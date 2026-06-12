//! `coregraph viz` — serve the bundled 3D graph viewer (atlas) over local
//! HTTP and proxy its API to the daemon IPC socket.
//!
//! This is the productized successor of the Node bridge (`viz/server.mjs`):
//! one command serves the viewer HTML (embedded at build time from
//! `viz/dist/index.html`), lists the projects loaded in daemon memory,
//! auto-starts the daemon when needed, and streams `export_graph` documents
//! straight out of daemon memory.
//!
//! Security model (localhost tooling, hardened against drive-by web pages):
//! the server binds to 127.0.0.1 only, rejects non-loopback Host headers
//! (DNS rebinding) and cross-origin requests, and requires a per-process
//! token — injected into the served HTML — on every `/api/*` call (CSRF).

use std::collections::HashMap;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, Request as HttpRequest, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Args;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{Mutex, OnceCell};

use crate::global_opts::GlobalOpts;
use crate::ipc;

/// Viewer HTML produced by `viz/dist/index.html` at build time (or a
/// self-describing placeholder when the npm build artifact was absent).
const EMBEDDED_HTML: &str = include_str!(concat!(env!("OUT_DIR"), "/atlas.html"));

const DAEMON_START_TIMEOUT: Duration = Duration::from_secs(30);
const DAEMON_START_POLL: Duration = Duration::from_millis(400);
/// export_graph may index a whole project before answering.
const GRAPH_TIMEOUT: Duration = Duration::from_secs(600);
/// Idle auto-stop forwarded to a daemon we spawn (matches `server start`).
const AUTO_STOP_MINUTES: u64 = 30;

#[derive(Args)]
pub struct VizArgs {
    /// HTTP port on 127.0.0.1.
    #[arg(long, default_value_t = 7321)]
    pub port: u16,

    /// Don't open the browser automatically.
    #[arg(long, default_value_t = false)]
    pub no_open: bool,

    /// Serve this HTML file instead of the embedded viewer (development).
    #[arg(long)]
    pub html: Option<PathBuf>,

    /// Run the atlas server in the background (detached from this terminal)
    /// and return once it answers on the port. Output goes to `viz.log` in
    /// the daemon runtime directory; stop it with `coregraph viz --stop`.
    #[arg(long, default_value_t = false, conflicts_with = "stop")]
    pub detach: bool,

    /// Stop the atlas server started with `--detach` (the most recent one).
    #[arg(long, default_value_t = false)]
    pub stop: bool,
}

type GraphCell = Arc<OnceCell<Arc<String>>>;

struct VizState {
    token: String,
    html: String,
    /// Host header values accepted as "this server" (loopback + port).
    allowed_hosts: Vec<String>,
    /// Project used when a daemon must be spawned and the request named none.
    default_project: PathBuf,
    /// Serializes daemon spawning so concurrent requests start it once.
    start_lock: Mutex<()>,
    /// Last `server restart` instant — concurrent unknown-method retries
    /// must not restart the daemon out from under each other.
    restart_gate: Mutex<Option<Instant>>,
    /// In-flight export_graph extractions keyed by project+confidence
    /// (single-flight: duplicate concurrent requests share one daemon call).
    inflight: Mutex<HashMap<String, GraphCell>>,
}

/// Errors mapped onto the same HTTP envelope the SPA already understands:
/// `{"error": "..."}` with 4xx/5xx status.
enum VizError {
    /// Bad client request → 4xx.
    Request(StatusCode, String),
    /// Daemon unreachable → 503.
    Down(String),
    /// Daemon answered with an error (or infra failure) → 502.
    Daemon(String),
}

impl IntoResponse for VizError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            VizError::Request(code, m) => (code, m),
            VizError::Down(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
            VizError::Daemon(m) => (StatusCode::BAD_GATEWAY, m),
        };
        (code, Json(json!({ "error": message }))).into_response()
    }
}

/// True when an IPC failure means "no daemon behind the socket".
fn is_conn_down(error: &anyhow::Error) -> bool {
    error
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .map(|io| {
            matches!(
                io.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
        })
        .unwrap_or(false)
}

struct DaemonState {
    running: bool,
    manager: Option<Value>,
}

async fn daemon_status() -> Result<DaemonState, VizError> {
    let resp = tokio::task::spawn_blocking(|| {
        ipc::send(&ipc::Request {
            method: "status".to_string(),
            params: Value::Null,
            project: PathBuf::new(),
        })
    })
    .await
    .map_err(|e| VizError::Daemon(format!("status task failed: {e}")))?;
    match resp {
        Ok(r) if r.ok => {
            let manager: Value = serde_json::from_str(&r.body)
                .map_err(|e| VizError::Daemon(format!("malformed daemon status: {e}")))?;
            Ok(DaemonState {
                running: true,
                manager: Some(manager),
            })
        }
        Ok(r) => Err(VizError::Daemon(
            r.error.unwrap_or_else(|| "daemon status error".to_string()),
        )),
        Err(e) if is_conn_down(&e) => Ok(DaemonState {
            running: false,
            manager: None,
        }),
        Err(e) => Err(VizError::Daemon(e.to_string())),
    }
}

/// Make sure a daemon is serving the socket, spawning one if needed.
/// Concurrent callers share a single spawn (start_lock + re-check).
async fn ensure_daemon(state: &VizState, project: &Path) -> Result<(DaemonState, bool), VizError> {
    let st = daemon_status().await?;
    if st.running {
        return Ok((st, false));
    }
    let _guard = state.start_lock.lock().await;
    let st = daemon_status().await?;
    if st.running {
        return Ok((st, false));
    }
    let spawn_project = project.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::daemon::spawn_background(&spawn_project, None, false, AUTO_STOP_MINUTES)
    })
    .await
    .map_err(|e| VizError::Daemon(format!("spawn task failed: {e}")))?
    .map_err(|e| VizError::Daemon(format!("daemon spawn failed: {e}")))?;

    let deadline = Instant::now() + DAEMON_START_TIMEOUT;
    while Instant::now() < deadline {
        tokio::time::sleep(DAEMON_START_POLL).await;
        let st = daemon_status().await?;
        if st.running {
            return Ok((st, true));
        }
    }
    Err(VizError::Daemon(format!(
        "daemon did not come up within {}s",
        DAEMON_START_TIMEOUT.as_secs()
    )))
}

/// Restart the daemon at most once per gate window — concurrent
/// unknown-method retries join the restart that already happened.
async fn restart_daemon_once(state: &VizState, project: &Path) -> Result<(), VizError> {
    let mut last = state.restart_gate.lock().await;
    if last
        .map(|t| t.elapsed() < Duration::from_secs(10))
        .unwrap_or(false)
    {
        return Ok(());
    }
    let restart_project = project.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::daemon::restart(&restart_project, None, false, AUTO_STOP_MINUTES)
    })
    .await
    .map_err(|e| VizError::Daemon(format!("restart task failed: {e}")))?
    .map_err(|e| VizError::Daemon(format!("daemon restart failed: {e}")))?;
    *last = Some(Instant::now());
    Ok(())
}

async fn export_graph_call(project: PathBuf, min_confidence: f64) -> Result<Arc<String>, VizError> {
    let resp = tokio::task::spawn_blocking(move || {
        ipc::send_with_timeout(
            &ipc::Request {
                method: "export_graph".to_string(),
                params: json!({ "min_confidence": min_confidence }),
                project,
            },
            Some(GRAPH_TIMEOUT),
        )
    })
    .await
    .map_err(|e| VizError::Daemon(format!("graph task failed: {e}")))?;
    match resp {
        Ok(r) if r.ok => Ok(Arc::new(r.body)),
        Ok(r) => Err(VizError::Daemon(
            r.error.unwrap_or_else(|| "export_graph failed".to_string()),
        )),
        Err(e) if is_conn_down(&e) => Err(VizError::Down(format!("daemon not reachable: {e}"))),
        Err(e) => Err(VizError::Daemon(e.to_string())),
    }
}

/// Forced full re-index: the daemon drops the cached graph and rebuilds from
/// source, bypassing the snapshot warm-load. Returns the daemon's
/// `{"reindexed":true,"symbols":N,"edges":M}` body.
async fn reload_project_call(project: PathBuf) -> Result<String, VizError> {
    let resp = tokio::task::spawn_blocking(move || {
        ipc::send_with_timeout(
            &ipc::Request {
                method: "reload_project".to_string(),
                params: Value::Null,
                project,
            },
            Some(GRAPH_TIMEOUT),
        )
    })
    .await
    .map_err(|e| VizError::Daemon(format!("reindex task failed: {e}")))?;
    match resp {
        Ok(r) if r.ok => Ok(r.body),
        Ok(r) => Err(VizError::Daemon(
            r.error.unwrap_or_else(|| "re-index failed".to_string()),
        )),
        Err(e) if is_conn_down(&e) => Err(VizError::Down(format!("daemon not reachable: {e}"))),
        Err(e) => Err(VizError::Daemon(e.to_string())),
    }
}

async fn fetch_graph_fresh(
    state: &VizState,
    project: PathBuf,
    min_confidence: f64,
) -> Result<Arc<String>, VizError> {
    ensure_daemon(state, &project).await?;
    match export_graph_call(project.clone(), min_confidence).await {
        Err(VizError::Daemon(message)) if message.contains("unknown method") => {
            // A daemon from an older binary is serving the socket; restart it
            // with the current binary and retry once.
            restart_daemon_once(state, &project).await?;
            ensure_daemon(state, &project).await?;
            export_graph_call(project, min_confidence).await
        }
        other => other,
    }
}

/// Single-flight wrapper: identical concurrent /api/graph requests share one
/// daemon extraction instead of racing several.
async fn fetch_graph(
    state: &Arc<VizState>,
    project: PathBuf,
    min_confidence: f64,
) -> Result<Arc<String>, VizError> {
    let key = format!("{} {min_confidence}", project.display());
    let cell: GraphCell = {
        let mut map = state.inflight.lock().await;
        map.entry(key.clone()).or_default().clone()
    };
    let result = cell
        .get_or_try_init(|| fetch_graph_fresh(state, project, min_confidence))
        .await
        .cloned();
    // Drop the cell so the next request fetches a fresh graph; waiters that
    // already hold the Arc still read the completed value.
    state.inflight.lock().await.remove(&key);
    result
}

fn resolve_project_path(input: &str) -> Value {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return json!({ "ok": false, "error": "empty path" });
    }
    let mut path = PathBuf::from(trimmed);
    if trimmed == "~" || trimmed.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            path = home.join(trimmed.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    match std::fs::canonicalize(&path) {
        Ok(canonical) if canonical.is_dir() => {
            json!({ "ok": true, "path": canonical.to_string_lossy() })
        }
        Ok(canonical) => json!({
            "ok": false,
            "error": format!("not a directory: {}", canonical.display())
        }),
        Err(_) => json!({
            "ok": false,
            "error": format!("path does not exist: {}", path.display())
        }),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────

async fn serve_html(State(state): State<Arc<VizState>>) -> Html<String> {
    // Inject the per-process token so only pages we served can call /api/*.
    let injected = state.html.replace(
        "</head>",
        &format!(
            "<script>window.__BRIDGE_TOKEN__=\"{}\"</script></head>",
            state.token
        ),
    );
    Html(injected)
}

async fn api_status() -> Result<Response, VizError> {
    let st = daemon_status().await?;
    Ok(Json(json!({
        "bridge": "ok",
        "socket": ipc::socket_path().to_string_lossy(),
        "running": st.running,
        "manager": st.manager,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct ResolveParams {
    #[serde(default)]
    path: String,
}

async fn api_resolve(
    params: Result<Query<ResolveParams>, QueryRejection>,
) -> Result<Response, VizError> {
    let Query(params) =
        params.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(resolve_project_path(&params.path)).into_response())
}

#[derive(Deserialize)]
struct StartBody {
    #[serde(default)]
    project: Option<String>,
}

async fn api_daemon_start(
    State(state): State<Arc<VizState>>,
    body: Result<Json<StartBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let project = body
        .project
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| state.default_project.clone());
    let (st, started) = ensure_daemon(&state, &project).await?;
    Ok(Json(json!({
        "running": st.running,
        "manager": st.manager,
        "started": started,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct GraphBody {
    project: String,
    #[serde(rename = "minConfidence", default)]
    min_confidence: Option<f64>,
}

async fn api_graph(
    State(state): State<Arc<VizState>>,
    body: Result<Json<GraphBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let resolved = resolve_project_path(&body.project);
    let Some(path) = resolved.get("path").and_then(Value::as_str) else {
        let message = resolved
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("invalid project path")
            .to_string();
        return Err(VizError::Request(StatusCode::BAD_REQUEST, message));
    };
    let min_confidence = body.min_confidence.filter(|c| c.is_finite()).unwrap_or(0.0);
    let graph = fetch_graph(&state, PathBuf::from(path), min_confidence).await?;
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        )],
        (*graph).clone(),
    )
        .into_response())
}

#[derive(Deserialize)]
struct ReindexBody {
    project: String,
}

async fn api_reindex(
    State(state): State<Arc<VizState>>,
    body: Result<Json<ReindexBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let resolved = resolve_project_path(&body.project);
    let Some(path) = resolved.get("path").and_then(Value::as_str) else {
        let message = resolved
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("invalid project path")
            .to_string();
        return Err(VizError::Request(StatusCode::BAD_REQUEST, message));
    };
    let project = PathBuf::from(path);
    ensure_daemon(&state, &project).await?;
    let body = match reload_project_call(project.clone()).await {
        Err(VizError::Daemon(message)) if message.contains("unknown method") => {
            // A daemon from an older binary is serving the socket; restart it
            // with the current binary and retry once.
            restart_daemon_once(&state, &project).await?;
            ensure_daemon(&state, &project).await?;
            reload_project_call(project).await?
        }
        other => other?,
    };
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        )],
        body,
    )
        .into_response())
}

/// Resolve a request's project string or fail with 400.
fn require_project(input: &str) -> Result<PathBuf, VizError> {
    let resolved = resolve_project_path(input);
    match resolved.get("path").and_then(Value::as_str) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err(VizError::Request(
            StatusCode::BAD_REQUEST,
            resolved
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("invalid project path")
                .to_string(),
        )),
    }
}

/// Proxy one analysis method to the daemon and return its JSON body.
/// `query`/`impact` answer symbol-not-found as ok:true with a PLAIN TEXT body
/// even in JSON mode — surface that as a 404-style error instead of letting
/// the browser choke on JSON.parse.
async fn daemon_json_call(
    state: &VizState,
    project: PathBuf,
    method: &'static str,
    params: Value,
) -> Result<String, VizError> {
    ensure_daemon(state, &project).await?;
    let resp = tokio::task::spawn_blocking(move || {
        ipc::send_with_timeout(
            &ipc::Request {
                method: method.to_string(),
                params,
                project,
            },
            Some(GRAPH_TIMEOUT),
        )
    })
    .await
    .map_err(|e| VizError::Daemon(format!("{method} task failed: {e}")))?;
    match resp {
        Ok(r) if r.ok => {
            let trimmed = r.body.trim_start();
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                Ok(r.body)
            } else {
                Err(VizError::Request(
                    StatusCode::NOT_FOUND,
                    r.body.trim().to_string(),
                ))
            }
        }
        Ok(r) => Err(VizError::Daemon(
            r.error.unwrap_or_else(|| format!("{method} failed")),
        )),
        Err(e) if is_conn_down(&e) => Err(VizError::Down(format!("daemon not reachable: {e}"))),
        Err(e) => Err(VizError::Daemon(e.to_string())),
    }
}

fn json_body_response(body: String) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        )],
        body,
    )
        .into_response()
}

#[derive(Deserialize)]
struct ImpactBody {
    project: String,
    symbol: String,
    #[serde(default)]
    depth: Option<u64>,
}

async fn api_impact(
    State(state): State<Arc<VizState>>,
    body: Result<Json<ImpactBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    if body.symbol.trim().is_empty() {
        return Err(VizError::Request(
            StatusCode::BAD_REQUEST,
            "missing \"symbol\"".to_string(),
        ));
    }
    let project = require_project(&body.project)?;
    let params = json!({
        "symbol": body.symbol,
        "depth": body.depth.unwrap_or(5),
        "risk": true,
        "output_format": "json",
    });
    let out = daemon_json_call(&state, project, "impact", params).await?;
    Ok(json_body_response(out))
}

#[derive(Deserialize)]
struct OrphansBody {
    project: String,
    #[serde(rename = "excludeTests", default)]
    exclude_tests: bool,
    #[serde(rename = "publicOnly", default = "default_true")]
    public_only: bool,
}

fn default_true() -> bool {
    true
}

async fn api_orphans(
    State(state): State<Arc<VizState>>,
    body: Result<Json<OrphansBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let project = require_project(&body.project)?;
    let params = json!({
        "exclude_tests": body.exclude_tests,
        "public_only": body.public_only,
        "output_format": "json",
    });
    let out = daemon_json_call(&state, project, "orphans", params).await?;
    Ok(json_body_response(out))
}

#[derive(Deserialize)]
struct InconsistenciesBody {
    project: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(rename = "excludeTests", default)]
    exclude_tests: bool,
}

async fn api_inconsistencies(
    State(state): State<Arc<VizState>>,
    body: Result<Json<InconsistenciesBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let project = require_project(&body.project)?;
    let mut params = json!({
        "exclude_tests": body.exclude_tests,
        "output_format": "json",
    });
    if let Some(category) = body.category.filter(|c| !c.is_empty()) {
        params["category"] = Value::String(category);
    }
    let out = daemon_json_call(&state, project, "inconsistencies", params).await?;
    Ok(json_body_response(out))
}

#[derive(Deserialize)]
struct DiffBody {
    project: String,
    #[serde(rename = "baseRef", default)]
    base_ref: Option<String>,
}

async fn api_diff(
    State(state): State<Arc<VizState>>,
    body: Result<Json<DiffBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let project = require_project(&body.project)?;
    let params = json!({
        "base_ref": body.base_ref.unwrap_or_else(|| "HEAD".to_string()),
    });
    let out = daemon_json_call(&state, project, "diff", params).await?;
    Ok(json_body_response(out))
}

#[derive(Deserialize)]
struct SourceBody {
    project: String,
    file: String,
    #[serde(rename = "spanStart", default)]
    span_start: usize,
    #[serde(rename = "spanEnd", default)]
    span_end: usize,
}

/// Lines of context shown around the requested span.
const SOURCE_CONTEXT_LINES: usize = 4;
/// Hard caps so a pathological request can't ship megabytes to the browser.
const SOURCE_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SOURCE_MAX_WINDOW_LINES: usize = 240;

/// Serve a source snippet for the code-preview panel. The file must resolve
/// (after symlinks) INSIDE the given project root — the token guard plus this
/// containment check keep the endpoint from becoming a generic file reader.
async fn api_source(
    State(state): State<Arc<VizState>>,
    body: Result<Json<SourceBody>, JsonRejection>,
) -> Result<Response, VizError> {
    let _ = &state; // guarded route; state unused beyond middleware
    let Json(body) = body.map_err(|e| VizError::Request(StatusCode::BAD_REQUEST, e.to_string()))?;
    let project = require_project(&body.project)?;
    if body.file.is_empty() {
        return Err(VizError::Request(
            StatusCode::BAD_REQUEST,
            "missing \"file\"".to_string(),
        ));
    }
    let canonical = std::fs::canonicalize(&body.file).map_err(|_| {
        VizError::Request(
            StatusCode::NOT_FOUND,
            format!("file does not exist: {}", body.file),
        )
    })?;
    if !canonical.starts_with(&project) {
        return Err(VizError::Request(
            StatusCode::FORBIDDEN,
            "file is outside the project root".to_string(),
        ));
    }
    let meta = std::fs::metadata(&canonical)
        .map_err(|e| VizError::Request(StatusCode::NOT_FOUND, e.to_string()))?;
    if !meta.is_file() {
        return Err(VizError::Request(
            StatusCode::BAD_REQUEST,
            "not a regular file".to_string(),
        ));
    }
    if meta.len() > SOURCE_MAX_FILE_BYTES {
        return Err(VizError::Request(
            StatusCode::PAYLOAD_TOO_LARGE,
            "file too large for preview".to_string(),
        ));
    }
    let bytes = std::fs::read(&canonical)
        .map_err(|e| VizError::Request(StatusCode::NOT_FOUND, e.to_string()))?;
    let text = String::from_utf8_lossy(&bytes);

    // Byte offsets → 0-based line numbers (spans come from the export data).
    let line_of = |offset: usize| -> usize {
        let clamped = offset.min(text.len());
        text.as_bytes()[..clamped]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    };
    let start_line = line_of(body.span_start);
    let end_line = line_of(body.span_end.max(body.span_start));

    let all_lines: Vec<&str> = text.lines().collect();
    let window_start = start_line.saturating_sub(SOURCE_CONTEXT_LINES);
    let window_end = (end_line + SOURCE_CONTEXT_LINES + 1)
        .min(all_lines.len())
        .min(window_start + SOURCE_MAX_WINDOW_LINES);
    let lines: Vec<&str> = all_lines
        .get(window_start..window_end)
        .unwrap_or(&[])
        .to_vec();

    Ok(Json(json!({
        "file": canonical.to_string_lossy(),
        "startLine": start_line,
        "endLine": end_line,
        "windowStart": window_start,
        "lines": lines,
        "truncated": window_end < (end_line + SOURCE_CONTEXT_LINES + 1).min(all_lines.len()),
    }))
    .into_response())
}

async fn fallback() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

// ── guard middleware ─────────────────────────────────────────────────────

fn forbidden(reason: &str) -> Response {
    (StatusCode::FORBIDDEN, format!("forbidden: {reason}")).into_response()
}

/// Reject DNS-rebound hosts, cross-origin requests, and tokenless /api calls
/// before any handler runs.
async fn guard(
    State(state): State<Arc<VizState>>,
    request: HttpRequest<Body>,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !state.allowed_hosts.iter().any(|h| h == host) {
        return forbidden("bad host");
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin_matches = origin
            .to_str()
            .ok()
            .and_then(|o| {
                o.strip_prefix("http://")
                    .or_else(|| o.strip_prefix("https://"))
            })
            .map(|origin_host| origin_host == host)
            .unwrap_or(false);
        if !origin_matches {
            return forbidden("bad origin");
        }
    }
    if request.uri().path().starts_with("/api/") {
        let token = request
            .headers()
            .get("x-bridge-token")
            .and_then(|v| v.to_str().ok());
        if !token.is_some_and(|t| token_matches(t, &state.token)) {
            return forbidden("missing bridge token");
        }
    }
    next.run(request).await
}

// ── wiring ───────────────────────────────────────────────────────────────

/// Per-process CSRF token: 256 bits straight from the OS CSPRNG.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS randomness unavailable");
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push_str(&format!("{byte:02x}"));
    }
    token
}

/// Constant-time token comparison — a byte-wise OR fold instead of `==`, so
/// the comparison time does not leak how many leading bytes matched. The
/// length check is fine to short-circuit: the token length is public.
fn token_matches(provided: &str, expected: &str) -> bool {
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn allowed_hosts(port: u16) -> Vec<String> {
    vec![
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
}

fn build_router(state: Arc<VizState>) -> Router {
    Router::new()
        .route("/", get(serve_html))
        .route("/index.html", get(serve_html))
        .route("/api/status", get(api_status))
        .route("/api/resolve", get(api_resolve))
        .route("/api/daemon/start", post(api_daemon_start))
        .route("/api/graph", post(api_graph))
        .route("/api/reindex", post(api_reindex))
        .route("/api/impact", post(api_impact))
        .route("/api/orphans", post(api_orphans))
        .route("/api/inconsistencies", post(api_inconsistencies))
        .route("/api/diff", post(api_diff))
        .route("/api/source", post(api_source))
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if result.is_err() {
        println!("  open {url} in your browser");
    }
}

/// PID file for the detached atlas server, holding `<pid> <port>`. One file —
/// only the most recently detached instance is tracked; a second `--detach`
/// (on another port) overwrites it, so stop the first one before starting a
/// second if both need lifecycle management.
fn viz_pid_path() -> PathBuf {
    crate::daemon::runtime_dir().join("viz.pid")
}

/// True when something is accepting TCP connections on 127.0.0.1:`port`.
fn port_serving(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Arguments for the detached child process: the same viz invocation minus
/// `--detach` (no recursion), plus `--no-open` (a headless child must not
/// pop a browser) and the canonical project root (the child must not depend
/// on this process's cwd surviving).
fn detached_child_args(args: &VizArgs, project: &Path) -> Vec<String> {
    let mut out = vec![
        "viz".to_string(),
        "--no-open".to_string(),
        "--port".to_string(),
        args.port.to_string(),
        "-C".to_string(),
        project.to_string_lossy().into_owned(),
    ];
    if let Some(html) = &args.html {
        out.push("--html".to_string());
        let abs = std::fs::canonicalize(html).unwrap_or_else(|_| html.clone());
        out.push(abs.to_string_lossy().into_owned());
    }
    out
}

/// `viz --detach`: re-spawn this binary as a detached background server, wait
/// until it answers on the port, and record `<pid> <port>` for `viz --stop`.
fn run_detached(args: &VizArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    if port_serving(args.port) {
        anyhow::bail!(
            "port {} is already serving — another atlas? Stop it (`coregraph viz --stop`) or pick a different --port",
            args.port
        );
    }
    let project =
        std::fs::canonicalize(&globals.project).unwrap_or_else(|_| globals.project.clone());
    let exe = std::env::current_exe().context("locating current executable")?;
    let mut cmd = Command::new(exe);
    cmd.args(detached_child_args(args, &project));

    let log_file = crate::daemon::runtime_dir().join("viz.log");
    if let Some(dir) = log_file.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating runtime dir {}", dir.display()))?;
    }
    let child =
        crate::daemon::spawn_detached(cmd, &log_file).context("spawning detached atlas server")?;
    std::fs::write(viz_pid_path(), format!("{} {}", child.id(), args.port))
        .with_context(|| format!("writing {}", viz_pid_path().display()))?;

    // The server binds before any heavy work, so readiness is quick; the
    // ceiling covers a cold filesystem or a busy machine.
    for _ in 0..50 {
        if port_serving(args.port) {
            let url = format!("http://127.0.0.1:{}/", args.port);
            println!("coregraph atlas (detached) → {url}");
            println!("  pid: {}   log: {}", child.id(), log_file.display());
            println!("  stop: coregraph viz --stop");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!(
        "detached atlas did not answer on port {} within 5s — see {}",
        args.port,
        log_file.display()
    )
}

/// `viz --stop`: terminate the instance recorded by the last `--detach`. A
/// pid file whose port is no longer serving is treated as stale and cleaned
/// up without signaling anything (the pid may have been recycled).
fn stop_detached() -> anyhow::Result<()> {
    let pid_file = viz_pid_path();
    let Ok(recorded) = std::fs::read_to_string(&pid_file) else {
        println!(
            "no detached atlas recorded ({} not found) — a foreground `coregraph viz` stops with Ctrl+C",
            pid_file.display()
        );
        return Ok(());
    };
    let mut parts = recorded.split_whitespace();
    let (Some(pid), Some(port)) = (
        parts.next().and_then(|s| s.parse::<u32>().ok()),
        parts.next().and_then(|s| s.parse::<u16>().ok()),
    ) else {
        let _ = std::fs::remove_file(&pid_file);
        anyhow::bail!(
            "malformed {} — removed; stop the server manually",
            pid_file.display()
        );
    };
    if !port_serving(port) {
        let _ = std::fs::remove_file(&pid_file);
        println!(
            "detached atlas (pid {pid}) is not serving on port {port} — cleaned up the stale record"
        );
        return Ok(());
    }
    crate::daemon::terminate_pid(pid)?;
    for _ in 0..50 {
        if !port_serving(port) {
            let _ = std::fs::remove_file(&pid_file);
            println!("stopped detached atlas (pid {pid}, port {port})");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("atlas (pid {pid}) is still serving on port {port} after the terminate signal — kill it manually")
}

pub fn run(args: VizArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    if args.stop {
        return stop_detached();
    }
    if args.detach {
        return run_detached(&args, globals);
    }
    let html = match &args.html {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading viewer HTML {}", path.display()))?,
        None => EMBEDDED_HTML.to_string(),
    };
    if html.contains("viewer not bundled") {
        eprintln!(
            "warning: this binary embeds no viewer (built without viz/dist/index.html); \
             serving a placeholder page"
        );
    }
    let default_project =
        std::fs::canonicalize(&globals.project).unwrap_or_else(|_| globals.project.clone());
    let state = Arc::new(VizState {
        token: random_token(),
        html,
        allowed_hosts: allowed_hosts(args.port),
        default_project,
        start_lock: Mutex::new(()),
        restart_gate: Mutex::new(None),
        inflight: Mutex::new(HashMap::new()),
    });
    let app = build_router(state);

    let runtime = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    runtime.block_on(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr} (is another atlas running?)"))?;
        let url = format!("http://127.0.0.1:{}/", args.port);
        println!("coregraph atlas → {url}");
        println!("  daemon socket: {}", ipc::socket_path().display());
        if !args.no_open {
            open_browser(&url);
        }
        axum::serve(listener, app).await.context("serving atlas")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    #[test]
    fn detached_child_args_rebuild_without_recursion() {
        let args = VizArgs {
            port: 7399,
            no_open: false,
            html: None,
            detach: true,
            stop: false,
        };
        let got = detached_child_args(&args, Path::new("/abs/proj"));
        assert_eq!(
            got,
            vec!["viz", "--no-open", "--port", "7399", "-C", "/abs/proj"]
        );
        assert!(
            !got.iter().any(|a| a == "--detach" || a == "--stop"),
            "the child must never re-detach or stop itself"
        );
    }

    #[test]
    fn detached_child_args_forward_html() {
        // A nonexistent path keeps canonicalize on its fallback branch, so
        // the assertion is deterministic on any machine.
        let args = VizArgs {
            port: 1,
            no_open: true,
            html: Some(PathBuf::from("/nope/dev.html")),
            detach: true,
            stop: false,
        };
        let got = detached_child_args(&args, Path::new("/p"));
        assert!(
            got.windows(2)
                .any(|w| w[0] == "--html" && w[1] == "/nope/dev.html"),
            "--html must be forwarded: {got:?}"
        );
    }

    fn test_state() -> Arc<VizState> {
        Arc::new(VizState {
            token: "testtoken".to_string(),
            html: "<html><head></head><body>atlas</body></html>".to_string(),
            allowed_hosts: allowed_hosts(7321),
            default_project: PathBuf::from("."),
            start_lock: Mutex::new(()),
            restart_gate: Mutex::new(None),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    fn request(method: &str, uri: &str) -> axum::http::request::Builder {
        HttpRequest::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "127.0.0.1:7321")
    }

    async fn body_string(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn rejects_bad_host() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/")
                    .header(header::HOST, "evil.example:7321")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_missing_host() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn rejects_cross_origin() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                request("GET", "/")
                    .header(header::ORIGIN, "https://attacker.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn allows_same_origin() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                request("GET", "/")
                    .header(header::ORIGIN, "http://127.0.0.1:7321")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn api_requires_token() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                request("GET", "/api/resolve?path=/tmp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn html_injects_token() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(request("GET", "/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("window.__BRIDGE_TOKEN__=\"testtoken\""));
    }

    #[tokio::test]
    async fn resolve_validates_paths() {
        let app = build_router(test_state());
        let dir = tempfile::tempdir().unwrap();
        let uri = format!("/api/resolve?path={}", dir.path().display());
        let resp = app
            .clone()
            .oneshot(
                request("GET", &uri)
                    .header("x-bridge-token", "testtoken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = serde_json::from_str(&body_string(resp).await).unwrap();
        assert_eq!(body["ok"], json!(true));

        let resp = app
            .oneshot(
                request("GET", "/api/resolve?path=/definitely/not/here")
                    .header("x-bridge-token", "testtoken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: Value = serde_json::from_str(&body_string(resp).await).unwrap();
        assert_eq!(body["ok"], json!(false));
    }

    #[tokio::test]
    async fn source_serves_snippet_inside_project() {
        let app = build_router(test_state());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn one() {}\nfn two() {}\nfn three() {}\n").unwrap();
        let payload = json!({
            "project": dir.path().to_string_lossy(),
            "file": file.to_string_lossy(),
            "spanStart": 12,
            "spanEnd": 22,
        });
        let resp = app
            .oneshot(
                request("POST", "/api/source")
                    .header("x-bridge-token", "testtoken")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = serde_json::from_str(&body_string(resp).await).unwrap();
        assert_eq!(body["startLine"], json!(1));
        assert!(body["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("fn two")));
    }

    #[tokio::test]
    async fn source_rejects_files_outside_project() {
        let app = build_router(test_state());
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "top secret").unwrap();
        // A plain outside path and a dot-dot traversal to a real system file
        // must both be refused with 403 (canonicalize resolves the dots).
        for file in [
            secret.to_string_lossy().to_string(),
            format!("{}/../../../../../../etc/hosts", project.path().display()),
        ] {
            let payload = json!({
                "project": project.path().to_string_lossy(),
                "file": file,
                "spanStart": 0,
                "spanEnd": 0,
            });
            let resp = app
                .clone()
                .oneshot(
                    request("POST", "/api/source")
                        .header("x-bridge-token", "testtoken")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::FORBIDDEN, "file: {file}");
        }
    }

    #[tokio::test]
    async fn impact_requires_symbol() {
        let app = build_router(test_state());
        let dir = tempfile::tempdir().unwrap();
        let payload = json!({ "project": dir.path().to_string_lossy(), "symbol": " " });
        let resp = app
            .oneshot(
                request("POST", "/api/impact")
                    .header("x-bridge-token", "testtoken")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn analysis_endpoints_validate_project() {
        let app = build_router(test_state());
        for path in [
            "/api/impact",
            "/api/orphans",
            "/api/inconsistencies",
            "/api/diff",
        ] {
            let payload = json!({ "project": "/definitely/not/here", "symbol": "x" });
            let resp = app
                .clone()
                .oneshot(
                    request("POST", path)
                        .header("x-bridge-token", "testtoken")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "endpoint: {path}");
        }
    }

    #[tokio::test]
    async fn graph_rejects_non_object_body() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(
                request("POST", "/api/graph")
                    .header("x-bridge-token", "testtoken")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("42"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_route_is_404() {
        let app = build_router(test_state());
        let resp = app
            .oneshot(request("GET", "/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn tokens_are_unique_and_hex() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

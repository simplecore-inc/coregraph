//! Minimal LSP stdio bridge. Implements `initialize`, `shutdown`, `exit`,
//! `textDocument/definition`, `textDocument/references`, and
//! `workspace/symbol` by delegating to the daemon over IPC (per
//! `docs/cli.md §10.4`). Auto-spawns the daemon on first use so IDE
//! integration is invisible — the IDE just runs `coregraph lsp`.

use crate::global_opts::GlobalOpts;
use clap::Args;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

#[derive(Args)]
pub struct LspArgs {
    /// Use stdio for transport. Accepted for compatibility with
    /// `vscode-languageclient`, which appends this flag when the
    /// extension sets `transport: TransportKind.stdio`. We always use
    /// stdio regardless, so this flag is effectively a no-op.
    #[arg(long, default_value_t = false, hide = true)]
    pub stdio: bool,
}

pub fn run(_args: LspArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    // Bring the daemon up. `ensure_running` returns `true` when the
    // socket is ready to accept queries; `false` means we couldn't
    // spawn it (binary missing, sandbox restriction) — in that case
    // each request falls back to a one-shot in-process build.
    let daemon_ready = crate::ipc::ensure_running(globals);
    let project = globals.project.clone();

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    loop {
        let msg = match read_lsp_message(&mut reader)? {
            Some(m) => m,
            None => break,
        };
        let parsed: Value = serde_json::from_str(&msg).unwrap_or(Value::Null);
        let method = parsed
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id = parsed.get("id").cloned();

        let response = match method.as_str() {
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "capabilities": {
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "workspaceSymbolProvider": true,
                    },
                    "serverInfo": {"name": "coregraph", "version": env!("CARGO_PKG_VERSION")},
                }
            })),
            "shutdown" => Some(json!({"jsonrpc":"2.0","id": id,"result": null})),
            "exit" => break,
            "textDocument/definition" => {
                Some(handle_definition(&parsed, id, &project, daemon_ready))
            }
            "textDocument/references" => {
                Some(handle_references(&parsed, id, &project, daemon_ready))
            }
            "workspace/symbol" => {
                Some(handle_workspace_symbol(&parsed, id, &project, daemon_ready))
            }
            _ => None, // ignore unknown notifications
        };
        if let Some(resp) = response {
            write_lsp_message(&mut writer, &resp.to_string())?;
        }
    }
    Ok(())
}

/// Route a `dispatch_cached` call either through the daemon (when
/// available — fast, cached graph) or directly in-process (cold build
/// per call, the pre-daemon behaviour). The dispatch path is identical
/// in both cases so handler output stays consistent.
fn route(
    method: &str,
    params: &Value,
    project: &std::path::Path,
    daemon_ready: bool,
) -> Option<Value> {
    if daemon_ready {
        let req = crate::ipc::Request {
            method: method.to_string(),
            params: params.clone(),
            project: project.to_path_buf(),
        };
        if let Ok(resp) = crate::ipc::send(&req) {
            if resp.ok {
                return serde_json::from_str(&resp.body).ok();
            }
        }
    }
    // Fallback: synchronous one-shot dispatch.
    let resp = crate::dispatch::dispatch(method, params, project);
    if !resp.ok {
        return None;
    }
    serde_json::from_str(&resp.body).ok()
}

fn read_lsp_message<R: BufRead>(reader: &mut R) -> anyhow::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        let header = header.trim_end_matches(&['\r', '\n'][..]);
        if header.is_empty() {
            break;
        }
        if let Some(v) = header.strip_prefix("Content-Length: ") {
            content_length = v.trim().parse().ok();
        }
    }
    let n = content_length.ok_or_else(|| anyhow::anyhow!("missing Content-Length"))?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn write_lsp_message<W: Write>(writer: &mut W, body: &str) -> anyhow::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()?;
    Ok(())
}

fn extract_word_at(
    uri: &str,
    line: usize,
    character: usize,
) -> Option<(String, std::path::PathBuf)> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let path = std::path::PathBuf::from(path);
    let src = std::fs::read_to_string(&path).ok()?;
    let line_text = src.lines().nth(line)?;
    let bytes = line_text.as_bytes();
    let mut start = character.min(bytes.len());
    let mut end = start;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some((
        String::from_utf8_lossy(&bytes[start..end]).into_owned(),
        path,
    ))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn handle_definition(
    req: &Value,
    id: Option<Value>,
    project: &std::path::Path,
    daemon_ready: bool,
) -> Value {
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let (name, _file) = match word_from_params(&params) {
        Some(p) => p,
        None => return json!({"jsonrpc":"2.0","id": id,"result": null}),
    };
    let body = route(
        "lsp.definition",
        &json!({ "symbol": name }),
        project,
        daemon_ready,
    );
    let locations = body
        .as_ref()
        .and_then(|b| b.get("locations").cloned())
        .unwrap_or_else(|| json!([]));
    let lsp_locations: Vec<Value> = locations
        .as_array()
        .map(|arr| arr.iter().map(loc_to_lsp_location).collect())
        .unwrap_or_default();
    json!({"jsonrpc":"2.0","id": id,"result": lsp_locations})
}

fn handle_references(
    req: &Value,
    id: Option<Value>,
    project: &std::path::Path,
    daemon_ready: bool,
) -> Value {
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let (name, _file) = match word_from_params(&params) {
        Some(p) => p,
        None => return json!({"jsonrpc":"2.0","id": id,"result": []}),
    };
    let body = route(
        "lsp.references",
        &json!({ "symbol": name }),
        project,
        daemon_ready,
    );
    let locations = body
        .as_ref()
        .and_then(|b| b.get("locations").cloned())
        .unwrap_or_else(|| json!([]));
    let lsp_locations: Vec<Value> = locations
        .as_array()
        .map(|arr| arr.iter().map(loc_to_lsp_location).collect())
        .unwrap_or_default();
    json!({"jsonrpc":"2.0","id": id,"result": lsp_locations})
}

fn handle_workspace_symbol(
    req: &Value,
    id: Option<Value>,
    project: &std::path::Path,
    daemon_ready: bool,
) -> Value {
    let query = req
        .get("params")
        .and_then(|p| p.get("query"))
        .and_then(|q| q.as_str())
        .unwrap_or("");
    let body = route(
        "lsp.workspace_symbol",
        &json!({ "query": query }),
        project,
        daemon_ready,
    );
    let symbols = body
        .as_ref()
        .and_then(|b| b.get("symbols").cloned())
        .unwrap_or_else(|| json!([]));
    let lsp_symbols: Vec<Value> = symbols
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    json!({
                        "name": s.get("name").cloned().unwrap_or(Value::Null),
                        "kind": symbol_kind_str_to_lsp(
                            s.get("kind").and_then(|k| k.as_str()).unwrap_or("")
                        ),
                        "location": loc_to_lsp_location(s),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"jsonrpc":"2.0","id": id,"result": lsp_symbols})
}

/// Convert a location object `{file, range, ...}` from the dispatch layer
/// into an LSP `Location` value `{uri, range}`.
///
/// The `range` key is pre-computed in `dispatch.rs` via `resolve_range()`.
/// Falls back to `(0,0)-(0,0)` for synthetic paths (e.g. test fixtures) where
/// the file does not exist on disk.
fn loc_to_lsp_location(loc: &Value) -> Value {
    let file = loc
        .get("file")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let range = loc.get("range").cloned().unwrap_or_else(|| {
        json!({
            "start": {"line": 0, "character": 0},
            "end":   {"line": 0, "character": 0},
        })
    });
    json!({
        "uri": format!("file://{}", file),
        "range": range,
    })
}

fn symbol_kind_str_to_lsp(name: &str) -> u32 {
    // LSP SymbolKind numeric codes — translated from the daemon's
    // string-form `Debug` rendering of `coregraph_core::SymbolKind`.
    // NOTE: relies on SymbolKind's Debug repr being stable. Prefer
    // Serialize before 1.0 so a variant rename cannot silently break
    // LSP kind classification across IDE clients.
    match name {
        "Function" => 12,
        "Method" => 6,
        "Class" => 5,
        "Struct" => 23,
        "Interface" => 11,
        "Trait" => 11,
        "Enum" => 10,
        "EnumVariant" => 22,
        "Constant" => 14,
        "Variable" => 13,
        "Field" => 8,
        "TypeAlias" => 5,
        "Module" => 2,
        "Namespace" => 3,
        // Doc comments / sections are textual content; LSP String (15) fits.
        "DocComment" => 15,
        "DocSection" => 15,
        _ => 1,
    }
}

fn word_from_params(params: &Value) -> Option<(String, std::path::PathBuf)> {
    let uri = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(|v| v.as_str())?;
    let pos = params.get("position")?;
    let line = pos.get("line")?.as_u64()? as usize;
    let character = pos.get("character")?.as_u64()? as usize;
    extract_word_at(uri, line, character)
}

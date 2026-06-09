//! IPC protocol: local socket JSON request/response between the thin
//! CLI client and the coregraph daemon.
//!
//! On Unix, the transport is a Unix-domain socket file.
//! On Windows, the transport is a named pipe (`\\.\pipe\coregraph-<user>`).
//! Both are abstracted by the `interprocess` crate's `local_socket` API.

use anyhow::{anyhow, Context, Result};
use interprocess::local_socket::{prelude::*, GenericFilePath, Stream};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
// `Duration` is only used for the Unix recv-timeout; Windows named pipes
// don't support per-stream timeouts, so the import is cfg-gated to keep
// the Windows build warning-free.
#[cfg(unix)]
use std::time::Duration;

/// Client-to-server request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Command name: "query", "impact", "orphans", ...
    pub method: String,
    /// Arbitrary command-specific JSON payload.
    pub params: serde_json::Value,
    /// Project root (absolute path) the daemon should operate against.
    pub project: PathBuf,
}

/// Server-to-client reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    /// Human/LLM/JSON-formatted body depending on request options.
    pub body: String,
    /// Optional structured error.
    pub error: Option<String>,
}

/// Resolve the IPC socket path (or named-pipe name on Windows).
///
/// On Unix: honors `$XDG_RUNTIME_DIR/coregraph/server.sock` when set,
/// else `~/.coregraph/server.sock`.
///
/// On Windows: returns a named-pipe path `\\.\pipe\coregraph-<USERNAME>`.
/// The `PathBuf` wrapper is used for a uniform return type; on Windows the
/// value is the pipe name string, not a real filesystem path.
pub fn socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            let p = PathBuf::from(xdg).join("coregraph");
            return p.join("server.sock");
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".coregraph")
            .join("server.sock")
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
        PathBuf::from(format!(r"\\.\pipe\coregraph-{user}"))
    }
    #[cfg(not(any(unix, windows)))]
    compile_error!("coregraph IPC transport only supports Unix and Windows targets");
}

/// Convert `socket_path()` to an interprocess `Name` suitable for
/// `Stream::connect` and `ListenerOptions::name`.
///
/// On Unix, this is a filesystem-path name (Unix-domain socket).
/// On Windows, the `\\.\pipe\...` string is passed through `to_fs_name`
/// which the `interprocess` Windows backend interprets as a named-pipe path.
pub fn socket_name() -> std::io::Result<interprocess::local_socket::Name<'static>> {
    socket_path().to_fs_name::<GenericFilePath>()
}

/// PID file path.
///
/// On Unix the PID file sits next to the socket file (both live under
/// `$XDG_RUNTIME_DIR/coregraph/` or `~/.coregraph/`). On Windows the
/// "socket path" is actually a named-pipe identifier (`\\.\pipe\...`)
/// and not a real filesystem entry, so the PID file has to live
/// somewhere writable instead — we use `%LOCALAPPDATA%\coregraph\`.
pub fn pid_path() -> PathBuf {
    #[cfg(unix)]
    {
        socket_path().with_file_name("server.pid")
    }
    #[cfg(windows)]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("coregraph")
            .join("server.pid")
    }
}

/// Returns true when the daemon socket currently accepts connections.
pub fn is_running() -> bool {
    socket_name().and_then(|name| Stream::connect(name)).is_ok()
}

/// Ensure a daemon is accepting IPC connections — spawning it in the
/// background if necessary — per `docs/cli.md §10.3`. Returns `Ok(true)`
/// when the daemon is (now) running, `Ok(false)` when auto-start was
/// suppressed and the daemon is absent, or `Err` when spawn failed.
///
/// Two opt-outs suppress the auto-start path: the `--no-auto-start`
/// CLI flag (threaded through `globals`) and the
/// `COREGRAPH_NO_AUTO_START` environment variable. The env var is
/// kept for shells / CI that can't append flags to every command.
pub fn ensure_running_or_spawn(project: &std::path::Path, allow_auto_start: bool) -> Result<bool> {
    if is_running() {
        return Ok(true);
    }
    if !allow_auto_start || std::env::var("COREGRAPH_NO_AUTO_START").is_ok() {
        return Ok(false);
    }
    // Delegate to the existing `daemon::spawn_background` helper. It
    // forks `coregraph server start --foreground`, redirects output to
    // `daemon.log`, and polls the socket up to 3s. On timeout it
    // returns an error — we propagate so the caller can fall back to
    // in-process instead of hanging the CLI.
    crate::daemon::spawn_background(project, None)?;
    Ok(is_running())
}

/// Non-failing convenience wrapper for thin-client command paths that
/// prefer "use daemon if possible, else in-process" semantics. Reads
/// `globals.no_auto_start` internally so each callsite doesn't have
/// to plumb the flag manually.
pub fn ensure_running(globals: &crate::global_opts::GlobalOpts) -> bool {
    let allow = !globals.no_auto_start;
    // Spawn against the ABSOLUTE project root. `spawn_background` forwards this
    // as `-C`, which becomes the daemon's pre-loaded default-project key. A
    // relative "." here would be canonicalized against the daemon's own cwd,
    // breaking the per-project routing the absolute paths the client now sends
    // depend on.
    let project = globals.project_root();
    match ensure_running_or_spawn(&project, allow) {
        Ok(up) => up,
        Err(e) => {
            eprintln!("[coregraph] daemon auto-start failed: {}", e);
            false
        }
    }
}

/// Send a single Request and await one Response (newline-delimited JSON).
pub fn send(request: &Request) -> Result<Response> {
    // Compute the socket identifier once; reuse it for both the connect
    // call (needs `Name`) and the error message (needs a displayable
    // path).
    let sock = socket_path();
    let name = sock
        .clone()
        .to_fs_name::<GenericFilePath>()
        .with_context(|| format!("resolving IPC socket name ({})", sock.display()))?;
    let mut stream = Stream::connect(name)
        .with_context(|| format!("connecting to IPC socket {}", sock.display()))?;
    // Windows named pipes do not support per-stream I/O timeouts at the
    // OS level; interprocess 2.x mirrors that by returning Unsupported
    // from `set_recv_timeout`. We keep the 30 s read cap on Unix where
    // the underlying Unix-domain socket honors it. On Windows the read
    // blocks until the peer replies or the pipe closes — acceptable for
    // Phase 0; revisit if we ever need a hard wall-clock cap there.
    #[cfg(unix)]
    stream.set_recv_timeout(Some(Duration::from_secs(30)))?;

    let line = serde_json::to_string(request)? + "\n";
    stream.write_all(line.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut buf = String::new();
    reader.read_line(&mut buf)?;
    let resp: Response = serde_json::from_str(buf.trim())
        .map_err(|e| anyhow!("malformed server reply: {} (raw: {})", e, buf))?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_non_empty() {
        let p = socket_path();
        assert!(p.to_str().is_some_and(|s| !s.is_empty()));
    }

    // Unix-only: the PID file lives next to the socket file because both
    // are real filesystem entries under the same runtime dir.
    #[test]
    #[cfg(unix)]
    fn pid_and_socket_share_directory() {
        assert_eq!(socket_path().parent(), pid_path().parent());
    }

    // Windows: the "socket path" is a named-pipe name, not a filesystem
    // entry, so the PID file cannot share its parent. Instead it must
    // land inside LOCALAPPDATA (or the fallback `.`) under a `coregraph`
    // subdir.
    #[test]
    #[cfg(windows)]
    fn windows_pid_path_is_under_local_data_dir() {
        let expected_root = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
        let p = pid_path();
        let parent = p.parent().expect("pid_path must have a parent directory");
        assert!(
            parent.starts_with(&expected_root),
            "pid_path parent {:?} should be under {:?}",
            parent,
            expected_root
        );
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("server.pid"));
    }

    #[test]
    fn request_roundtrip_json() {
        let req = Request {
            method: "query".to_string(),
            params: serde_json::json!({"name": "Foo"}),
            project: PathBuf::from("/tmp/x"),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(back.method, "query");
    }

    #[test]
    #[cfg(windows)]
    fn windows_pipe_path_format_is_well_formed() {
        let p = socket_path();
        let s = p.to_string_lossy();
        assert!(
            s.starts_with(r"\\.\pipe\coregraph-"),
            "expected named-pipe format, got: {s}"
        );
    }
}

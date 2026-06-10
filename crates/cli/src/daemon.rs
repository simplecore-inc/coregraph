//! Daemon process management — launch, stop, status, health-check.
//! The daemon is the same `coregraph` binary launched with
//! `coregraph server start --foreground` (internal). When started without
//! `--foreground`, the binary forks into the background.

use crate::ipc;
use anyhow::{anyhow, Context, Result};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Rotation threshold for `daemon.log`. 5 MiB is large enough to keep
/// one busy session's output in a single file (each request typically
/// contributes a few hundred bytes) but small enough that `tail -f`
/// stays responsive and disk creep is bounded.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// Snapshot of daemon state returned by `server status`.
#[derive(Debug, serde::Serialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub socket: PathBuf,
    pub version: &'static str,
}

/// Check the daemon's current state.
pub fn status() -> DaemonStatus {
    let pid = std::fs::read_to_string(ipc::pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());

    DaemonStatus {
        running: ipc::is_running(),
        pid,
        socket: ipc::socket_path(),
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Resolve the directory used for the daemon's PID file, log file, and
/// (on Unix) the socket file.
///
/// On Unix the runtime dir lives next to the socket file. On Windows the
/// "socket path" is a named-pipe identifier (`\\.\pipe\...`) — not a real
/// filesystem path — so we derive the runtime dir from the PID file path
/// instead, which `ipc::pid_path()` already resolves to a real directory
/// (`%LOCALAPPDATA%\coregraph\`).
fn runtime_dir() -> PathBuf {
    #[cfg(unix)]
    {
        ipc::socket_path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
    #[cfg(windows)]
    {
        ipc::pid_path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Spawn the daemon in the background (detached). Returns immediately.
/// The child writes its PID into the pid-file and listens on the IPC socket.
pub fn spawn_background(
    project: &std::path::Path,
    http: Option<&str>,
    allow_external: bool,
    auto_stop_minutes: u64,
) -> Result<()> {
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating daemon runtime dir {}", dir.display()))?;

    if ipc::is_running() {
        return Ok(());
    }

    // Auto-create `<project>/.coregraph/config.toml` with defaults on
    // first daemon launch so thin-client commands (which trigger this
    // path) leave behind a discoverable per-project config.
    crate::commands::config::ensure_local_default(project);

    let exe = std::env::current_exe().context("locating current executable")?;
    let mut cmd = Command::new(exe);
    cmd.args(["server", "start", "--foreground"]);
    cmd.args(["-C", project.to_string_lossy().as_ref()]);
    if let Some(h) = http {
        cmd.args(["--http", h]);
    }
    // Forward the bind-scope and idle auto-stop settings to the foreground
    // child. Without this, `server start --allow-external` failed the child's
    // non-localhost bind gate and `--auto-stop-minutes` (including `0` to
    // disable) was silently dropped on the detached path.
    if allow_external {
        cmd.arg("--allow-external");
    }
    cmd.args(["--auto-stop-minutes", &auto_stop_minutes.to_string()]);

    // Detach from the controlling terminal so the child outlives the parent.
    // On Unix: call setsid() in the child before exec to create a new session.
    // On Windows: set CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS creation flags,
    // which are the Win32 equivalent — the child gets no console and no inherited
    // session. No `unsafe` needed on Windows; `creation_flags` is a safe method.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP = 0x00000200, DETACHED_PROCESS = 0x00000008:
        // detach from the parent's console and give the child an independent
        // process group (the Windows setsid() analog).
        //
        // CREATE_BREAKAWAY_FROM_JOB = 0x01000000: a launcher — notably a CI
        // runner — often wraps the step's process tree in a job object with
        // kill-on-close. Without breakaway the *detached* daemon stays in that
        // job and is reaped the moment the spawning process / job ends, so it
        // dies seconds after `server start` returns. Breaking away keeps it
        // alive. Some job objects forbid breakaway (CreateProcess then fails);
        // the spawn below retries without this flag in that case.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    }

    cmd.stdin(std::process::Stdio::null());

    let log_dir = &dir; // same dir on both platforms; already created above
    let log_file = log_dir.join("daemon.log");
    // Rotate before reusing the log handle so a long-running daemon
    // doesn't accumulate tens of MB of stderr indefinitely. We keep
    // exactly one previous log (`daemon.log.1`) — one prior
    // generation is enough for post-mortem diagnostics, and rolling
    // multiple files complicates downstream tools.
    rotate_log_if_large(&log_file, MAX_LOG_BYTES);
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
    {
        cmd.stdout(f.try_clone()?);
        cmd.stderr(f);
    } else {
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
    }

    let child = cmd.spawn();
    // If CREATE_BREAKAWAY_FROM_JOB was rejected (the parent job object forbids
    // breakaway), CreateProcess fails; retry detached without that flag.
    #[cfg(windows)]
    let child = match child {
        Ok(c) => Ok(c),
        Err(_) => {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x00000008 | 0x00000200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
            cmd.spawn()
        }
    };
    let child = child.context("spawning daemon process")?;
    let _ = std::fs::write(ipc::pid_path(), child.id().to_string());

    for _ in 0..30 {
        if ipc::is_running() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!(
        "daemon failed to start within timeout — see {}",
        log_file.display()
    ))
}

/// Stop the running daemon by sending SIGTERM (Unix) or via taskkill (Windows).
pub fn stop() -> Result<()> {
    let pid_file = ipc::pid_path();
    let pid_str = std::fs::read_to_string(&pid_file)
        .with_context(|| format!("reading {}", pid_file.display()))?;
    let pid_str = pid_str.trim();

    // On Unix: send SIGTERM via libc::kill for graceful shutdown.
    // On Windows: use `taskkill /PID <pid> /F` (forceful terminate).
    // /F is required because Windows has no SIGTERM equivalent that the
    // daemon can intercept — the process is terminated immediately. This
    // matches the Phase 0.5d scope (same philosophy as setsid: delegate
    // cleanup to the OS). Revisit for graceful shutdown in a later phase.
    #[cfg(unix)]
    {
        let pid: i32 = pid_str
            .parse()
            .map_err(|_| anyhow!("invalid PID in {}", pid_file.display()))?;
        unsafe {
            if libc::kill(pid, libc::SIGTERM) != 0 {
                return Err(anyhow!(
                    "SIGTERM to {} failed: {}",
                    pid,
                    std::io::Error::last_os_error()
                ));
            }
        }
    }
    #[cfg(windows)]
    {
        // pid is parsed and validated before being passed as an argument, so
        // there is no shell-injection risk (it's a plain decimal integer).
        let pid: u32 = pid_str
            .parse()
            .map_err(|_| anyhow!("invalid PID in {}", pid_file.display()))?;
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()
            .map_err(|e| anyhow!("taskkill spawn failed: {}", e))?;
        if !status.success() {
            return Err(anyhow!("taskkill exited with {}", status));
        }
    }

    for _ in 0..30 {
        if !ipc::is_running() {
            let _ = std::fs::remove_file(&pid_file);
            // On Unix the socket is a real filesystem file that must be
            // cleaned up. On Windows the "socket path" is a named-pipe
            // identifier (`\\.\pipe\...`) — not a filesystem entry — so
            // remove_file would fail; skip it on Windows.
            #[cfg(unix)]
            let _ = std::fs::remove_file(ipc::socket_path());
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(anyhow!("daemon did not shut down within 3 s"))
}

/// Restart = stop (best-effort) + start (current-dir project).
pub fn restart(
    project: &std::path::Path,
    http: Option<&str>,
    allow_external: bool,
    auto_stop_minutes: u64,
) -> Result<()> {
    if ipc::is_running() {
        stop().ok();
    }
    spawn_background(project, http, allow_external, auto_stop_minutes)
}

/// Rotate `log_path` → `log_path.1` when the current file exceeds
/// `max_bytes`. The previous `.1` is overwritten — we keep exactly
/// two generations to bound disk usage at `2 * MAX_LOG_BYTES` while
/// still preserving the immediately previous run for post-mortems.
/// Silent on errors: rotation is best-effort, never blocks daemon
/// startup.
fn rotate_log_if_large(log_path: &Path, max_bytes: u64) {
    let Ok(meta) = std::fs::metadata(log_path) else {
        return;
    };
    if meta.len() <= max_bytes {
        return;
    }
    let backup = log_path.with_extension("log.1");
    let _ = std::fs::rename(log_path, backup);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_returns_struct() {
        let s = status();
        assert!(!s.version.is_empty());
    }

    /// Verify the Win32 API constants used for process detachment haven't
    /// drifted from their documented values. This is a compile-time sanity
    /// check only — the arithmetic is trivial but the hex values are load-bearing.
    #[test]
    #[cfg(windows)]
    fn windows_creation_flags_are_well_known_constants() {
        // Win32 API constant values used by spawn_background's creation_flags.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        assert_eq!(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS, 0x208);
        assert_eq!(CREATE_BREAKAWAY_FROM_JOB, 0x0100_0000);
    }
}

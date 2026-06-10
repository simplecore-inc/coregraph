//! End-to-end daemon lifecycle test: spawn -> IPC health -> stop.
//!
//! Runs on both Unix (socket file) and Windows (named pipe) via the
//! cross-platform transport introduced in Phase 0.5d (`interprocess` 2.x).
//!
//! WARNING: these tests stop any coregraph daemon running under the
//! current user. Do not run while an IDE is actively using the daemon.
//!
//! IMPORTANT: Both tests manipulate the shared user-level socket, so they are
//! marked `#[serial]` (serial_test) to force sequential execution even under
//! the default parallel test runner — parallel execution would interfere.
//!
//! ## Exit code quirks (discovered during 0.5d-2)
//!
//! `coregraph server status` always exits 0 — even when the daemon is stopped.
//! `coregraph server stop` always exits 0 — even when no daemon is running.
//! The only reliable signal is the JSON body: `{"running":true/false,...}`.
//! All poll predicates therefore parse JSON from `server status --json`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serial_test::serial;

/// Returns the path to the compiled `coregraph` binary.
/// Cargo sets `CARGO_BIN_EXE_coregraph` for integration tests when the crate
/// declares a `[[bin]]` target with that name.
fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_coregraph"))
}

/// Query the daemon's running state by parsing `server status --json`.
/// Returns `true` when the daemon is up and listening, `false` on any error
/// or when `running` is false/absent.
fn is_daemon_running() -> bool {
    Command::new(binary_path())
        .args(["server", "status", "--json"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("running")?.as_bool())
        .unwrap_or(false)
}

/// Poll a predicate up to `max` duration, sleeping `step` between checks.
/// Returns `true` if the predicate became true before the deadline.
fn poll_until<F: FnMut() -> bool>(mut f: F, max: Duration, step: Duration) -> bool {
    let deadline = Instant::now() + max;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(step);
    }
}

/// Best-effort cleanup: stop any daemon already running under the user-level
/// socket. Ignores errors — a clean environment (no prior daemon) is the
/// expected starting state in most runs.
fn stop_existing_daemon() {
    let _ = Command::new(binary_path())
        .args(["server", "stop"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Give the OS a moment to release the socket / pipe handle.
    thread::sleep(Duration::from_millis(500));
}

/// Full lifecycle: spawn daemon -> verify running via IPC -> stop -> verify stopped.
///
/// The IPC round-trip is validated by `server status --json` returning
/// `"running":true` while the daemon is up. That status call goes through
/// the IPC socket to the daemon's `"status"` handler (see server.rs lines
/// 119-128) — not just a PID file check — so it proves the transport works.
#[test]
#[serial]
fn daemon_starts_then_responds_to_health_then_stops() {
    stop_existing_daemon();

    // Create a temporary project directory. The daemon requires `-C <project>`
    // to know what to index.
    let tmp = std::env::temp_dir().join(format!(
        "cg-daemon-lifecycle-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    // -- Phase 1: Start the daemon ----------------------------------------
    //
    // `server start` (without `--foreground`) calls `daemon::spawn_background`
    // which detaches the child, polls the socket for up to 3s, then returns.
    // `.status()` waits for the spawner to exit — not for the daemon's
    // lifetime — so this returns quickly.
    let start_status = Command::new(binary_path())
        .args(["-C", tmp.to_string_lossy().as_ref(), "server", "start"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to spawn `coregraph server start`");
    assert!(
        start_status.success(),
        "`coregraph server start` exited with {}",
        start_status
    );

    // `spawn_background` already polls for up to 3 s, but give extra margin
    // for slow CI machines (Windows in particular) before we start our own
    // polling loop.
    let ready = poll_until(
        is_daemon_running,
        Duration::from_secs(20),
        Duration::from_millis(250),
    );
    assert!(ready, "daemon did not become ready within 20s");

    // -- Phase 2: IPC health check ----------------------------------------
    //
    // `server status --json` sends an IPC `"status"` request to the daemon
    // when `running` is true. The round-trip through the socket proves the
    // transport is traversable — not just that the socket file exists.
    // `is_daemon_running()` above already exercised this, so we just assert
    // the cached state here as a belt-and-suspenders check.
    assert!(
        is_daemon_running(),
        "daemon became unresponsive between ready-poll and health check"
    );

    // -- Phase 3: Stop the daemon -----------------------------------------
    let stop_status = Command::new(binary_path())
        .args(["server", "stop"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to spawn `coregraph server stop`");
    assert!(
        stop_status.success(),
        "`coregraph server stop` exited with {}",
        stop_status
    );

    // -- Phase 4: Verify the daemon is actually down ----------------------
    let stopped = poll_until(
        || !is_daemon_running(),
        Duration::from_secs(10),
        Duration::from_millis(250),
    );
    assert!(stopped, "daemon did not shut down within 10s of stop");

    // Cleanup.
    std::fs::remove_dir_all(&tmp).ok();
}

/// Stopping when no daemon is running should complete gracefully (exit 0).
///
/// Note: `coregraph server stop` always exits 0, even when no daemon was
/// running — it prints "Daemon not running" and returns. This test confirms
/// that behavior is stable and does not regress into a panic or hang.
#[test]
#[serial]
fn stop_when_no_daemon_running_completes_gracefully() {
    stop_existing_daemon();

    // No daemon is running. `server stop` should succeed without hanging.
    let status = Command::new(binary_path())
        .args(["server", "stop"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to spawn `coregraph server stop`");
    assert!(
        status.success(),
        "`coregraph server stop` unexpectedly failed with {} when no daemon was running",
        status
    );

    // Confirm nothing started as a side effect.
    assert!(
        !is_daemon_running(),
        "a daemon appeared after `server stop` on an idle system"
    );
}

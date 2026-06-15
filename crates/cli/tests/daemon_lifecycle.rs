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

/// Generate a project large enough that its initial index runs for several
/// seconds — long enough to outlast the round-trip timeout in the test below.
/// Returns the project root (a fresh temp directory the caller must clean up).
#[cfg(unix)]
fn make_indexing_heavy_project(file_count: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "cg-index-heavy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
    ));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..file_count {
        let mut body = String::new();
        for j in 0..8 {
            body.push_str(&format!(
                "def func_{i}_{j}(x):\n    return helper_{i}_{k}(x) + {i} + {j}\n\n",
                k = (j + 1) % 8
            ));
            body.push_str(&format!(
                "def helper_{i}_{j}(y):\n    return y * {i} - {j}\n\n"
            ));
        }
        std::fs::write(src.join(format!("mod_{i}.py")), body).unwrap();
    }
    root
}

/// Replicate `ipc::socket_path()` for the test process. The cli crate exposes
/// no library target, so an integration test cannot import the helper — it must
/// resolve the same path the daemon binds.
#[cfg(unix)]
fn user_socket_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(xdg).join("coregraph").join("server.sock");
    }
    PathBuf::from(std::env::var("HOME").expect("HOME must be set on unix"))
        .join(".coregraph")
        .join("server.sock")
}

/// Send a single `status` request over the daemon socket and return the raw
/// response line, bounded by `timeout`. A timeout maps to an `io::Error` so the
/// caller can distinguish "no reply in time" from a successful round-trip.
#[cfg(unix)]
fn status_roundtrip(sock: &std::path::Path, timeout: Duration) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(sock)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(b"{\"method\":\"status\",\"params\":null,\"project\":\"\"}\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

/// Regression guard for the bind-before-accept dead window: the daemon used to
/// run its initial index synchronously *before* entering the accept loop, so a
/// client that connected while a large project was indexing was accepted into
/// the listen backlog but got no reply until indexing finished. The thin client
/// hit its receive timeout and surfaced "Resource temporarily unavailable
/// (os error 35)" (the atlas bridge rendered it as "bridge error").
///
/// After the fix the accept loop starts first and the index runs on a
/// background thread, so a `status` round-trip succeeds while the project is
/// still indexing.
#[test]
#[serial]
#[cfg(unix)]
fn daemon_accept_loop_serves_status_during_initial_index() {
    use std::os::unix::net::UnixStream;

    stop_existing_daemon();
    assert!(
        poll_until(
            || !is_daemon_running(),
            Duration::from_secs(10),
            Duration::from_millis(100)
        ),
        "a pre-existing daemon would not stop"
    );

    let proj = make_indexing_heavy_project(2000);

    // Run the daemon in the foreground as a child: it binds the socket and then
    // begins indexing immediately — exactly the window we want to probe.
    let mut child = Command::new(binary_path())
        .args([
            "-C",
            proj.to_string_lossy().as_ref(),
            "server",
            "start",
            "--foreground",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn foreground daemon");

    let sock = user_socket_path();
    // Wait only until the socket is *connectable* (bound), NOT until indexing
    // finishes — connect succeeds against the listen backlog the moment the
    // socket binds.
    let bound = poll_until(
        || UnixStream::connect(&sock).is_ok(),
        Duration::from_secs(15),
        Duration::from_millis(20),
    );

    let outcome = if bound {
        status_roundtrip(&sock, Duration::from_secs(2))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "socket never bound",
        ))
    };

    // Tear the daemon down hard so we don't wait out the still-running index
    // through graceful shutdown.
    let _ = child.kill();
    let _ = child.wait();
    let _ = Command::new(binary_path())
        .args(["server", "stop"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    std::fs::remove_dir_all(&proj).ok();

    let line = outcome.expect(
        "status round-trip timed out while the daemon was indexing — the IPC \
         accept loop is blocked behind the initial index (bind-before-accept \
         regression)",
    );
    let resp: serde_json::Value =
        serde_json::from_str(line.trim()).expect("daemon status reply must be JSON");
    assert_eq!(
        resp.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "status reply not ok during initial index: {line}"
    );
}

/// Regression guard for duplicate-daemon prevention: several `server start`
/// commands firing at once (e.g. multiple MCP / LSP / viz bridges auto-spawning
/// the daemon on a fresh boot) must collapse to exactly ONE running daemon, and
/// every loser must exit cleanly (status 0). Before the singleton flock the
/// losers raced `remove_file` + `bind` and died with `Address already in use`
/// (non-zero), sometimes orphaning a second live daemon that held a graph in
/// memory but no socket.
///
/// Runs on an isolated socket via a temp `XDG_RUNTIME_DIR` so it neither
/// touches nor is disturbed by the user's real daemon.
#[test]
#[serial]
#[cfg(unix)]
fn concurrent_server_starts_collapse_to_single_daemon() {
    let xdg = std::env::temp_dir().join(format!(
        "cg-singleton-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos(),
    ));
    std::fs::create_dir_all(xdg.join("coregraph")).unwrap();
    let proj = make_indexing_heavy_project(1); // tiny: one file, fast to index

    const N: usize = 6;
    let entries: Vec<std::process::Child> = (0..N)
        .map(|_| {
            Command::new(binary_path())
                .args([
                    "-C",
                    proj.to_string_lossy().as_ref(),
                    "server",
                    "start",
                    "--foreground",
                ])
                .env("XDG_RUNTIME_DIR", &xdg)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn foreground daemon")
        })
        .collect();
    let mut entries: Vec<(std::process::Child, Option<std::process::ExitStatus>)> =
        entries.into_iter().map(|c| (c, None)).collect();

    // Poll until only the single winner is still alive (or the deadline).
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut alive = 0;
        for (child, exit) in entries.iter_mut() {
            if exit.is_none() {
                match child.try_wait() {
                    Ok(Some(status)) => *exit = Some(status),
                    Ok(None) => alive += 1,
                    Err(_) => {}
                }
            }
        }
        if alive <= 1 || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Tally survivors and loser exit statuses, then tear everything down before
    // asserting so a failure never leaks a daemon.
    let mut alive_count = 0;
    let mut loser_statuses: Vec<std::process::ExitStatus> = Vec::new();
    for (child, exit) in entries.iter_mut() {
        match exit {
            Some(status) => loser_statuses.push(*status),
            None => {
                alive_count += 1;
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    std::fs::remove_dir_all(&proj).ok();
    std::fs::remove_dir_all(&xdg).ok();

    assert_eq!(
        alive_count, 1,
        "exactly one daemon must survive concurrent starts; {alive_count} were still running"
    );
    assert_eq!(
        loser_statuses.len(),
        N - 1,
        "expected {} losing daemons to have exited",
        N - 1
    );
    for status in &loser_statuses {
        assert!(
            status.success(),
            "a losing daemon exited with failure ({status}) — it hit the \
             Address-already-in-use race instead of a clean singleton-lock exit"
        );
    }
}

//! `coregraph server ...` — daemon lifecycle.

use crate::daemon;
use crate::global_opts::GlobalOpts;
use crate::ipc;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct ServerArgs {
    #[command(subcommand)]
    pub command: ServerCommand,
}

#[derive(Subcommand)]
pub enum ServerCommand {
    /// Start the daemon (detached by default).
    Start(ServerStartArgs),
    /// Stop the running daemon (SIGTERM + drain).
    Stop,
    /// Show daemon status.
    Status {
        /// Emit JSON instead of human text.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Stop + start in one command.
    Restart(ServerStartArgs),
    /// Register the daemon as an OS service (launchd on macOS, systemd on Linux).
    Install,
    /// Remove the OS service registration.
    Uninstall,
}

#[derive(Args, Clone)]
pub struct ServerStartArgs {
    /// Expose an additional HTTP API at this address (e.g. 127.0.0.1:9120).
    /// Passing `--http` without a value binds to the default
    /// `127.0.0.1:27787`. The port is deliberately off the common
    /// 8080/8000/3000 band to avoid clashes with local dev servers.
    #[arg(long, num_args = 0..=1, default_missing_value = DEFAULT_HTTP_ADDR)]
    pub http: Option<String>,

    /// Allow binding to non-localhost interfaces.
    #[arg(long, default_value_t = false)]
    pub allow_external: bool,

    /// Run in the foreground (the process is the daemon itself).
    #[arg(long, default_value_t = false)]
    pub foreground: bool,

    /// Minutes of full idleness (no loaded projects, no in-flight
    /// queries) before the daemon self-terminates. Default 30.
    /// Pass `0` to disable auto-stop entirely.
    #[arg(long, default_value_t = 30)]
    pub auto_stop_minutes: u64,
}

/// Default address when `--http` is passed without an argument.
/// `27787` mirrors the digits in "coregraph" and is outside both the
/// IANA well-known range (<1024) and every common web-dev default
/// (8000/8080/3000/5000). Change with care — bind changes ripple to
/// every client integration.
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:27787";

pub fn run(args: ServerArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    match args.command {
        ServerCommand::Start(sa) => start(sa, globals),
        ServerCommand::Stop => stop(),
        ServerCommand::Status { json } => print_status(json),
        ServerCommand::Restart(sa) => restart(sa, globals),
        ServerCommand::Install => install(globals),
        ServerCommand::Uninstall => uninstall(),
    }
}

fn start(sa: ServerStartArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    if sa.foreground {
        // We ARE the daemon now.
        run_foreground(
            &globals.project,
            sa.http.as_deref(),
            sa.allow_external,
            sa.auto_stop_minutes,
        )
    } else {
        if ipc::is_running() {
            println!("Daemon already running at {}", ipc::socket_path().display());
            return Ok(());
        }
        daemon::spawn_background(&globals.project, sa.http.as_deref())?;
        println!("Started daemon — socket: {}", ipc::socket_path().display());
        Ok(())
    }
}

fn stop() -> anyhow::Result<()> {
    if !ipc::is_running() {
        println!("Daemon not running");
        return Ok(());
    }
    daemon::stop()?;
    println!("Daemon stopped");
    Ok(())
}

fn restart(sa: ServerStartArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    daemon::restart(&globals.project, sa.http.as_deref())?;
    println!("Daemon restarted");
    Ok(())
}

fn print_status(as_json: bool) -> anyhow::Result<()> {
    let s = daemon::status();
    // Query the daemon's ProjectManager via IPC for the enriched snapshot.
    let projects = if s.running {
        let req = ipc::Request {
            method: "status".to_string(),
            params: serde_json::Value::Null,
            project: PathBuf::new(),
        };
        ipc::send(&req)
            .ok()
            .filter(|r| r.ok)
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.body).ok())
    } else {
        None
    };

    if as_json {
        let mut out = serde_json::json!({
            "running": s.running,
            "pid": s.pid,
            "socket": s.socket.display().to_string(),
            "version": s.version,
        });
        if let Some(p) = projects {
            out["manager"] = p;
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("daemon: {}", if s.running { "RUNNING" } else { "STOPPED" });
        println!("version: {}", s.version);
        println!("socket: {}", s.socket.display());
        if let Some(pid) = s.pid {
            println!("pid: {}", pid);
        }
        if let Some(manager) = projects {
            let loaded = manager
                .get("projects")
                .and_then(|p| p.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let max = manager
                .get("max_loaded")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let uptime = manager
                .get("uptime_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!("uptime: {}s", uptime);
            println!("Projects ({}/{} loaded):", loaded, max);
            if let Some(arr) = manager.get("projects").and_then(|p| p.as_array()) {
                for p in arr {
                    let path = p
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<unknown>");
                    let loaded = p.get("loaded").and_then(|v| v.as_bool()).unwrap_or(false);
                    let loading = p.get("loading").and_then(|v| v.as_bool()).unwrap_or(false);
                    let nodes = p.get("node_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let edges = p.get("edge_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let idle = p.get("idle_seconds").and_then(|v| v.as_u64()).unwrap_or(0);
                    let active = p
                        .get("active_queries")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let status = if loading {
                        "LOADING"
                    } else if loaded {
                        "ACTIVE"
                    } else {
                        "UNLOADED"
                    };
                    println!(
                        "  [{}] {} — {} symbols, {} edges, idle {}s, {} in-flight",
                        status, path, nodes, edges, idle, active
                    );
                }
            }
        }
    }
    Ok(())
}

fn install(globals: &GlobalOpts) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let project = std::fs::canonicalize(&globals.project)?;
    #[cfg(target_os = "macos")]
    {
        install_launchd(&exe, &project)
    }
    #[cfg(target_os = "linux")]
    {
        install_systemd(&exe, &project)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (exe, project);
        Err(anyhow::anyhow!(
            "service install is only supported on macOS and Linux"
        ))
    }
}

fn uninstall() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        uninstall_launchd()
    }
    #[cfg(target_os = "linux")]
    {
        uninstall_systemd()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(anyhow::anyhow!(
            "service uninstall is only supported on macOS and Linux"
        ))
    }
}

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents/com.coregraph.daemon.plist")
}

#[cfg(target_os = "macos")]
fn install_launchd(exe: &std::path::Path, project: &std::path::Path) -> anyhow::Result<()> {
    let path = launchd_plist_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.coregraph.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>server</string>
    <string>start</string>
    <string>--foreground</string>
    <string>-C</string>
    <string>{project}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/coregraph-daemon.log</string>
  <key>StandardErrorPath</key><string>/tmp/coregraph-daemon.err</string>
</dict>
</plist>
"#,
        exe = exe.display(),
        project = project.display(),
    );
    std::fs::write(&path, content)?;
    println!("Installed launchd plist: {}", path.display());
    println!("  Run: launchctl load {}", path.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_launchd() -> anyhow::Result<()> {
    let path = launchd_plist_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Removed {}", path.display());
    } else {
        println!("No plist found at {}", path.display());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("systemd/user/coregraph.service")
}

#[cfg(target_os = "linux")]
fn install_systemd(exe: &std::path::Path, project: &std::path::Path) -> anyhow::Result<()> {
    let path = systemd_unit_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let content = format!(
        "[Unit]\nDescription=CoreGraph daemon\nAfter=default.target\n\n\
[Service]\nExecStart={exe} server start --foreground -C {project}\nRestart=on-failure\n\n\
[Install]\nWantedBy=default.target\n",
        exe = exe.display(),
        project = project.display(),
    );
    std::fs::write(&path, content)?;
    println!("Installed systemd unit: {}", path.display());
    println!("  Run: systemctl --user enable --now coregraph");
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_systemd() -> anyhow::Result<()> {
    let path = systemd_unit_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Removed {}", path.display());
    } else {
        println!("No unit found at {}", path.display());
    }
    Ok(())
}

/// Foreground daemon: own the socket, serve requests until SIGTERM.
fn run_foreground(
    project: &std::path::Path,
    http: Option<&str>,
    allow_external: bool,
    auto_stop_minutes: u64,
) -> anyhow::Result<()> {
    use interprocess::local_socket::ListenerOptions;
    use std::io::{BufRead, BufReader, Write};

    // Security gate: HTTP binding outside loopback requires the
    // explicit opt-in. Previously the `--allow-external` flag was
    // accepted but ignored (parameter was prefixed `_`), which meant
    // any user passing `--http 0.0.0.0:27787` got an unauthenticated
    // service on every interface. Enforce the gate before we touch
    // the socket.
    if let Some(addr) = http {
        if !is_loopback_bind(addr) && !allow_external {
            anyhow::bail!(
                "refusing to bind HTTP on non-loopback address '{}' without `--allow-external` \
                 (the server is unauthenticated; expose on 127.0.0.1 / localhost / ::1 only, \
                 or pass `--allow-external` to override)",
                addr
            );
        }
    }

    // Direct `coregraph server start --foreground` invocations bypass
    // `daemon::spawn_background`, so the auto-init helper is also
    // anchored here. Idempotent — no-op if the file already exists.
    crate::commands::config::ensure_local_default(project);

    let sock = ipc::socket_path();
    // Ensure the parent directories for both the socket and the PID
    // file exist. On Unix they are the same directory (e.g.
    // `$XDG_RUNTIME_DIR/coregraph/`), so the second call is a no-op.
    // On Windows `sock.parent()` is `\\.\pipe`, which is *not* a real
    // filesystem path — creating it would fail, so we skip it and only
    // ensure the PID path's parent (`%LOCALAPPDATA%\coregraph\`).
    #[cfg(unix)]
    {
        if let Some(dir) = sock.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    if let Some(dir) = ipc::pid_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Before clearing the socket file, check whether another daemon
    // is alive on it. Two processes simultaneously running
    // `server start` (e.g. `coregraph lsp` auto-spawning its daemon
    // while an IDE extension concurrently spawns its own) used to
    // stomp on each other: the later process's `remove_file` + bind
    // would unlink the earlier process's socket inode, leaving it
    // with a dead listener while the new one took the path. Clients
    // then hit the new daemon (no loaded graph) or got timeouts
    // depending on the race timing. Probe first and bail out if the
    // socket already accepts connections; the existing daemon owns
    // the project.
    if ipc::is_running() {
        eprintln!(
            "coregraph daemon: another instance is already bound to {}, exiting",
            sock.display()
        );
        return Ok(());
    }
    // On Unix the socket is a real filesystem entry; remove any stale
    // corpse file before binding so we don't hit AddrInUse. On Windows
    // named pipes are kernel objects with no filesystem entry, so
    // remove_file would fail — skip it entirely.
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&sock);
    }
    let listener = ListenerOptions::new()
        .name(ipc::socket_name()?)
        .create_sync()?;

    // Advertise a readable-only PID that callers can `kill`.
    std::fs::write(ipc::pid_path(), std::process::id().to_string())?;

    // Optional HTTP bridge (reuses coregraph-server).
    if let Some(addr) = http {
        let addr = addr.to_string();
        let project_owned = project.to_path_buf();
        std::thread::spawn(move || {
            // Index once at startup so /query and /health return real counts.
            let graph = match coregraph_extractor::build_graph(&project_owned) {
                Ok((g, _)) => g,
                Err(e) => {
                    eprintln!("HTTP bridge disabled — index failed: {}", e);
                    return;
                }
            };
            let state = coregraph_server::AppState::with_graph(graph);
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                let _ = coregraph_server::serve(&addr, state).await;
            });
        });
    }

    // Multi-project runtime. The default project (the one the daemon was
    // launched for) is pre-loaded synchronously so the first query against
    // it is cached. Other projects are loaded on demand when an IPC request
    // names a different `request.project`.
    use crate::project_manager::{ProjectManager, ProjectManagerConfig};
    use std::sync::{Arc, Mutex};
    let project = std::fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    // Config-file knobs (project-local over global) seed the lifecycle
    // defaults; the `--auto-stop-minutes` CLI flag still wins for auto_stop.
    // `0` disables auto-stop by pushing the threshold past anything
    // reachable in practice (~584 years), keeping the sweep loop identical.
    let overrides = crate::commands::config::server_overrides(&project);
    let defaults = ProjectManagerConfig::default();
    let config = ProjectManagerConfig {
        max_loaded: overrides.max_loaded_projects.unwrap_or(defaults.max_loaded),
        max_loaded_bytes: overrides
            .max_loaded_bytes
            .unwrap_or(defaults.max_loaded_bytes),
        idle_unload: overrides
            .idle_unload_minutes
            .map(|m| std::time::Duration::from_secs(m * 60))
            .unwrap_or(defaults.idle_unload),
        auto_stop: if auto_stop_minutes == 0 {
            std::time::Duration::from_secs(u64::MAX / 2)
        } else {
            std::time::Duration::from_secs(auto_stop_minutes * 60)
        },
    };
    let graceful_shutdown =
        std::time::Duration::from_secs(overrides.graceful_shutdown_sec.unwrap_or(30));
    let manager = Arc::new(ProjectManager::new(config));
    // Shared heal tracker — persists across IPC requests so the second
    // query after an on-disk edit sees a hash mismatch and re-parses.
    let heal_tracker: Arc<Mutex<coregraph_graph::FileStateTracker>> =
        Arc::new(Mutex::new(coregraph_graph::FileStateTracker::new()));

    eprintln!("coregraph daemon indexing {} ...", project.display());
    let indexed = std::time::Instant::now();
    let default_graph = manager.get_or_load::<_, anyhow::Error>(&project, load_cached_graph)?;
    manager.release(&project);
    {
        let g = default_graph.read().unwrap();
        eprintln!(
            "coregraph daemon ready — {} symbols, {} edges ({}ms), socket {}",
            g.node_count(),
            g.edge_count(),
            indexed.elapsed().as_millis(),
            sock.display()
        );
        // Seed the heal tracker with the initial file set so the very first
        // IPC request doesn't treat every evidence file as "newly changed".
        // Without this, `filter_real_changes` on a fresh tracker reports
        // every file as changed and the heal loop re-extracts everything,
        // duplicating every node already in the cached graph.
        let initial_files: std::collections::HashSet<std::path::PathBuf> =
            g.nodes().map(|n| n.file.to_path_buf()).collect();
        let mut tracker = heal_tracker.lock().unwrap();
        let _ = tracker.filter_real_changes(initial_files.iter());
    }

    // Background watcher: rebuild the default project when its files change.
    // Attach the project's PathExcluder to the watcher so build outputs
    // (target/, build/), dependency caches, and .git/ never wake the daemon.
    // Without this filter a `cargo build` would emit thousands of events
    // and stall every query behind an endless incremental rebuild loop.
    {
        let manager = manager.clone();
        let project_w = project.clone();
        std::thread::spawn(move || {
            let excluder = coregraph_query::PathExcluder::from_project_root(&project_w);
            let Ok(watcher) = coregraph_watcher::FileWatcher::with_filter(&project_w, move |p| {
                !excluder.is_excluded(p)
            }) else {
                eprintln!(
                    "[daemon] watcher failed to start for {} — file-change reindex disabled",
                    project_w.display()
                );
                return;
            };
            eprintln!("[daemon] watching {} for changes", project_w.display());
            loop {
                let paths = watcher.receiver.next_changed_files();
                if paths.is_empty() {
                    // `next_changed_files()` blocks while the watcher is healthy,
                    // but returns an empty batch *immediately* once its channel is
                    // closed or erroring. A bare `continue` then busy-spins a CPU
                    // core — on a constrained (e.g. Windows CI) runner this starves
                    // the IPC accept loop and the daemon looks dead. Back off so a
                    // degenerate watcher can never peg a core.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                // Incremental rebuild path: invalidate and re-extract only
                // the changed files, then re-run cross-file resolution on
                // the full graph. Falls back to a full rebuild if the
                // project isn't loaded yet (first-touch) or the
                // incremental path errors out.
                let paths_vec: Vec<std::path::PathBuf> = paths.to_vec();
                let project_dir = project_w.clone();
                let incremental = (|| -> anyhow::Result<()> {
                    let existing_arc =
                        manager.get_or_load::<_, anyhow::Error>(&project_dir, load_cached_graph)?;
                    {
                        let mut guard = existing_arc.write().unwrap();
                        let _ = coregraph_extractor::build_graph_incremental(
                            &project_dir,
                            &mut guard,
                            &paths_vec,
                        );
                    }
                    // The graph was mutated in place — flag it dirty so the
                    // next eviction persists a fresh snapshot.
                    manager.mark_dirty(&project_dir);
                    manager.release(&project_dir);
                    Ok(())
                })();
                if incremental.is_ok() {
                    eprintln!(
                        "[daemon] incremental rebuild after {} change(s) in {}",
                        paths.len(),
                        project_w.display()
                    );
                } else if let Ok((g, _)) = coregraph_extractor::build_graph(&project_w) {
                    manager.unload(&project_w);
                    let built_at = std::time::SystemTime::now();
                    let _ = manager.get_or_load::<_, std::convert::Infallible>(&project_w, |_| {
                        Ok(crate::project_manager::BuiltGraph {
                            graph: g,
                            built_at,
                            from_snapshot: false,
                        })
                    });
                    manager.release(&project_w);
                    eprintln!(
                        "[daemon] full rebuild (incremental failed) after {} change(s) in {}",
                        paths.len(),
                        project_w.display()
                    );
                }
            }
        });
    }

    // Background idle sweeper (runs every 60s). Three stages:
    //   1. `sweep_idle` drops individual projects that exceeded
    //      `idle_unload`, freeing their graphs from memory.
    //   2. `gc_loaded` reaps `Gone`-marked symbols past `GONE_GC_TTL` from
    //      every still-loaded graph, reclaiming tombstoned nodes/edges.
    //   3. `should_auto_stop` checks if the daemon as a whole has been
    //      quiet for `auto_stop` (30 min by default). When true the
    //      sweeper self-terminates via SIGTERM so the signal handler
    //      installed below runs the same cleanup path as `server stop`.
    {
        let manager = manager.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let dropped = manager.sweep_idle();
            if dropped > 0 {
                eprintln!("[daemon] idle sweep unloaded {} project(s)", dropped);
            }
            let reaped = manager.gc_loaded(coregraph_graph::GONE_GC_TTL);
            if reaped > 0 {
                eprintln!("[daemon] gc reaped {} gone symbol(s)", reaped);
            }
            if manager.should_auto_stop() {
                eprintln!("[daemon] auto-stop: all projects idle, shutting down");
                // Trigger graceful shutdown directly by setting the same flag the
                // SIGTERM/SIGINT handler sets: the poller thread then drains
                // in-flight queries, persists dirty graphs, and exits. Done this
                // way rather than self-SIGTERM because Windows has no `libc::kill`
                // equivalent, and setting the flag reaches the identical path.
                SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
                return;
            }
        });
    }

    // Install SIGTERM/SIGINT handlers so launchd / Ctrl-C terminate cleanly.
    //
    // Graceful shutdown:
    //   1. Signal handler sets a global atomic flag (signal-safe).
    //   2. A dedicated poller thread checks the flag and, when set, waits
    //      up to `graceful_shutdown` (default 30s, `server.graceful_shutdown_sec`)
    //      for in-flight queries to drain before exiting. This gives
    //      excalidraw-size rebuilds time to finish writing their snapshot
    //      before the process goes away.
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    static SHUTDOWN: AtomicBool = AtomicBool::new(false);
    // Unix delivers SIGTERM/SIGINT to request graceful shutdown; the handler
    // flips SHUTDOWN and the poller below drains in-flight work and exits.
    // Windows has no equivalent delivery — the daemon is force-stopped with
    // `taskkill /F` — and installing CRT signal handlers on a detached,
    // console-less process is both pointless and a plausible source of spurious
    // early shutdowns, so gate them to Unix. SHUTDOWN stays cross-platform: the
    // idle auto-stop sets it on both platforms.
    #[cfg(unix)]
    extern "C" fn on_term(_: libc::c_int) {
        SHUTDOWN.store(true, AtomicOrdering::SeqCst);
    }
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as usize);
        libc::signal(libc::SIGINT, on_term as *const () as usize);
    }
    {
        let manager = manager.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if !SHUTDOWN.load(AtomicOrdering::SeqCst) {
                    continue;
                }
                let deadline = std::time::Instant::now() + graceful_shutdown;
                loop {
                    let in_flight: usize = manager
                        .status()
                        .projects
                        .iter()
                        .map(|p| p.active_queries)
                        .sum();
                    if in_flight == 0 || std::time::Instant::now() >= deadline {
                        // Persist dirty graphs so unsaved mutations survive the
                        // exit and the next daemon start warm-loads them.
                        let persisted = manager.persist_all_dirty();
                        if persisted > 0 {
                            eprintln!("[daemon] persisted {} graph(s) on shutdown", persisted);
                        }
                        if in_flight > 0 {
                            eprintln!(
                                "[daemon] shutdown timeout: {} in-flight queries aborted",
                                in_flight
                            );
                        } else {
                            eprintln!("[daemon] graceful shutdown complete");
                        }
                        std::process::exit(0);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        });
    }

    // Each accepted connection runs on its own OS thread. Previously
    // the loop processed connections serially, so a long query
    // (e.g. `diff` on a 15K-symbol monorepo takes several seconds)
    // head-of-line-blocked every subsequent request — including cheap
    // `health` probes — and the extension saw 30-second IPC timeouts.
    // Mutation safety is unchanged: `ProjectManager` holds graphs in
    // `Arc<RwLock>`; readers run concurrently, writers exclusively.
    for conn in listener {
        let Ok(stream) = conn else { continue };
        let manager = manager.clone();
        let project = project.clone();
        let heal_tracker = heal_tracker.clone();
        std::thread::spawn(move || {
            let _ = (|| -> anyhow::Result<()> {
                let mut stream = stream;
                // Read the one-line request by *borrowing* the stream rather than
                // `try_clone()`: duplicating a half-closed Windows named-pipe
                // handle crashed the whole daemon process on the first
                // connect-and-drop probe (`is_running()`), before any handler log.
                // The protocol is one request → one response per connection, so a
                // borrowed BufReader (dropped before we write) loses nothing.
                let mut line = String::new();
                {
                    let mut reader = BufReader::new(&mut stream);
                    if reader.read_line(&mut line).is_err() {
                        return Ok(());
                    }
                }
                let request: ipc::Request = match serde_json::from_str(line.trim()) {
                    Ok(r) => r,
                    Err(e) => {
                        let resp = ipc::Response {
                            ok: false,
                            body: String::new(),
                            error: Some(format!("invalid request JSON: {}", e)),
                        };
                        let _ = writeln!(stream, "{}", serde_json::to_string(&resp)?);
                        return Ok(());
                    }
                };
                // Per-request trace — essential for diagnosing "daemon hung"
                // reports. Shows which project path the client addressed, so
                // an extension sending the wrong CWD (e.g. `/` after forgetting
                // to set ServerOptions.cwd) is immediately visible in the log.
                eprintln!(
                    "[daemon] request method={} project={}",
                    request.method,
                    request.project.display()
                );

                // Special method: `status` returns a multi-project summary.
                if request.method == "status" {
                    let status = manager.status();
                    let reply = ipc::Response {
                        ok: true,
                        body: serde_json::to_string(&status).unwrap_or_default(),
                        error: None,
                    };
                    let _ = writeln!(stream, "{}", serde_json::to_string(&reply)?);
                    return Ok(());
                }

                // Requests targeting this project reuse the cached graph; any other
                // (absolute) project path triggers get_or_load on the ProjectManager.
                // A relative path is rejected rather than silently resolved against
                // the daemon's own cwd — see `resolve_target_project`.
                let target_project = match resolve_target_project(&request.project, &project) {
                    Ok(p) => p,
                    Err(msg) => {
                        let resp = ipc::Response {
                            ok: false,
                            body: String::new(),
                            error: Some(msg),
                        };
                        let _ = writeln!(stream, "{}", serde_json::to_string(&resp)?);
                        return Ok(());
                    }
                };
                // Reindex performs a surgical update reading the file directly from
                // disk, so the staleness of other source files is irrelevant. Skip
                // the source_tree_is_newer mtime check to avoid a full rebuild that
                // would preempt the fast-path before dispatch_reindex_mutable runs.
                // All other methods use the normal refresh-on-stale path.
                let graph_arc = match if request.method == "reindex" {
                    manager.get_or_load_without_refresh::<_, anyhow::Error>(
                        &target_project,
                        load_cached_graph,
                    )
                } else {
                    manager.get_or_load::<_, anyhow::Error>(&target_project, load_cached_graph)
                } {
                    Ok(g) => g,
                    Err(e) => {
                        let resp = ipc::Response {
                            ok: false,
                            body: String::new(),
                            error: Some(format!("project load failed: {}", e)),
                        };
                        let _ = writeln!(stream, "{}", serde_json::to_string(&resp)?);
                        return Ok(());
                    }
                };

                // On-demand healing: before dispatching, check whether any file
                // known to the cached graph has a stale content hash. We skip
                // healing when the request carried a no_heal flag. The healing
                // runs under a 200ms budget; files that exceed it are left stale
                // and the query proceeds on the pre-heal graph.
                let no_heal = request
                    .params
                    .get("no_heal")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let heal_report = if !no_heal && request.method == "query" {
                    let files_snapshot: Vec<std::path::PathBuf> = {
                        let g = graph_arc.read().unwrap();
                        g.nodes()
                            .map(|n| n.file.to_path_buf())
                            .collect::<std::collections::HashSet<_>>()
                            .into_iter()
                            .collect()
                    };
                    // Use the long-lived tracker to find genuinely changed files,
                    // then re-extract them under the 200ms budget.
                    let mut tracker = heal_tracker.lock().unwrap();
                    let change_batch = tracker.filter_real_changes(files_snapshot.iter());
                    drop(tracker);

                    if change_batch.is_empty() {
                        None
                    } else {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_millis(200);
                        let mut healed = Vec::new();
                        let mut stale = Vec::new();
                        let mut g = graph_arc.write().unwrap();
                        for path in &change_batch.changed {
                            if std::time::Instant::now() >= deadline {
                                stale.push(path.clone());
                                continue;
                            }
                            let source = match std::fs::read_to_string(path) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            for extractor in coregraph_extractor::all_extractors() {
                                if coregraph_extractor::scanner::extension_matches(
                                    path,
                                    extractor.file_extensions(),
                                ) {
                                    let _ = extractor.extract(path, &source, &mut g);
                                    break;
                                }
                            }
                            healed.push(path.clone());
                        }
                        Some(coregraph_graph::HealingReport {
                            unchanged: Vec::new(),
                            healed,
                            stale_after_timeout: stale,
                            removed: change_batch.removed.clone(),
                        })
                    }
                } else {
                    None
                };

                // Healing re-extracted changed files into the cached graph; flag it
                // dirty so the next eviction persists the healed state rather than a
                // stale snapshot.
                if heal_report
                    .as_ref()
                    .map(|r| !r.healed.is_empty())
                    .unwrap_or(false)
                {
                    manager.mark_dirty(&target_project);
                }

                let reply = if request.method == "reindex" {
                    // Mutable path — acquire write guard (briefly blocks all readers).
                    // Reindex IS the freshness update, so skip healing entirely.
                    let mut g = graph_arc.write().unwrap();
                    crate::dispatch::dispatch_reindex_mutable(
                        &request.params,
                        &mut g,
                        &target_project,
                    )
                } else if request.method == "diff" {
                    // Git-enriched diff path — read-only graph access, but needs the
                    // project root for git operations. Bypasses the healing banner
                    // because it has its own response shape.
                    let g = graph_arc.read().unwrap();
                    crate::dispatch::dispatch_diff_with_git(&request.params, &g, &target_project)
                } else if request.method == "orphans" {
                    // Orphans needs the project root to classify library-vs-application
                    // packages (manifest-derived), so its public API surface is not
                    // mislabelled as dead code. Mirrors the CLI local path.
                    let g = graph_arc.read().unwrap();
                    crate::dispatch::cached_orphans(&request.params, &g, Some(&target_project))
                } else {
                    // Read-only path — preserve existing healing banner logic.
                    let g = graph_arc.read().unwrap();
                    let mut reply =
                        crate::dispatch::dispatch_cached(&request.method, &request.params, &g);
                    if let Some(report) = heal_report {
                        if !report.stale_after_timeout.is_empty() {
                            // The banner has to land somewhere that won't corrupt
                            // a JSON response body. When the body already looks
                            // like JSON (starts with '{' or '['), attach the
                            // healing notice under a `_warnings` field so
                            // downstream parsers still see valid JSON. Otherwise
                            // prepend the text line as before.
                            let trimmed = reply.body.trim_start();
                            let banner = format!(
                                "⚠ healing in progress for {} file(s)",
                                report.stale_after_timeout.len()
                            );
                            let parsed_json =
                                if trimmed.starts_with('{') || trimmed.starts_with('[') {
                                    serde_json::from_str::<serde_json::Value>(trimmed).ok()
                                } else {
                                    None
                                };
                            if let Some(mut val) = parsed_json {
                                if let Some(obj) = val.as_object_mut() {
                                    let warnings = obj
                                        .entry("_warnings")
                                        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                                    if let Some(arr) = warnings.as_array_mut() {
                                        arr.push(serde_json::Value::String(banner.clone()));
                                    }
                                }
                                reply.body = serde_json::to_string_pretty(&val)
                                    .unwrap_or_else(|_| format!("{banner}\n{}", reply.body));
                            } else {
                                reply.body = format!("{banner}\n{}", reply.body);
                            }
                        }
                    }
                    reply
                };
                if request.method == "reindex" {
                    // dispatch_reindex_mutable updated the cached graph in place.
                    manager.mark_dirty(&target_project);
                }
                manager.release(&target_project);
                let _ = writeln!(stream, "{}", serde_json::to_string(&reply)?);
                Ok(())
            })();
        });
    }
    Ok(())
}

/// Load a project's graph for the daemon cache.
///
/// First tries a warm load from `.coregraph/snapshot.bin`: if a snapshot
/// exists and no source file is newer than the time it was built, the graph
/// is restored from disk without re-running tree-sitter extraction. Otherwise
/// (cold start, stale snapshot, or unreadable file) it falls back to the
/// canonical `build_graph` path shared with every other CLI command.
///
/// `built_at` for a fresh build is captured BEFORE the extraction walk, so a
/// file edited during the build is still caught as stale on the next access.
fn load_cached_graph(p: &std::path::Path) -> anyhow::Result<crate::project_manager::BuiltGraph> {
    use crate::project_manager::{source_tree_is_newer, BuiltGraph};

    let snap = coregraph_graph::snapshot_path(p);
    if let Some(warm) =
        coregraph_graph::warm_load(&snap, |built_at| !source_tree_is_newer(p, built_at))
    {
        return Ok(BuiltGraph {
            graph: warm.graph,
            built_at: warm.built_at,
            from_snapshot: true,
        });
    }

    let built_at = std::time::SystemTime::now();
    let graph = crate::graph_loader::load_project_graph_only(p)?;
    Ok(BuiltGraph {
        graph,
        built_at,
        from_snapshot: false,
    })
}

/// True if `addr` binds only to the local loopback interface. Accepts
/// the three conventional encodings: IPv4 `127.x.x.x`, hostname
/// `localhost`, and IPv6 `[::1]`. Everything else is treated as
/// network-exposed and gated behind `--allow-external`.
fn is_loopback_bind(addr: &str) -> bool {
    // Strip the port suffix so we match `127.0.0.1:27787` correctly.
    // We split from the right: `[::1]:27787` has colons in the host
    // part, but the last `:` is still the port separator.
    let host = match addr.rsplit_once(':') {
        Some((h, _)) => h,
        None => addr,
    };
    // Unwrap IPv6 literal brackets if present.
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    host == "127.0.0.1" || host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// Resolve the project a daemon request targets, enforcing the absolute-path
/// contract declared on `ipc::Request::project`.
///
/// - **Empty path** → the project the daemon was launched for (its pre-loaded
///   default graph). Preserves the existing "no project specified" behavior.
/// - **Relative path** → rejected with an error. `std::fs::canonicalize` would
///   resolve it against the DAEMON's working directory — frozen at first spawn,
///   not the client's cwd — and silently serve the wrong project. Returning an
///   error surfaces the client bug instead of hiding it behind a
///   plausible-but-wrong answer.
/// - **Absolute path** → canonicalized so symlinks / `..` are normalized to the
///   same key `ProjectManager` stores the graph under.
fn resolve_target_project(requested: &Path, daemon_default: &Path) -> Result<PathBuf, String> {
    if requested.as_os_str().is_empty() {
        return Ok(daemon_default.to_path_buf());
    }
    if requested.is_relative() {
        return Err(format!(
            "relative project path {} rejected: the client must send an absolute path. \
             A relative path resolves against the daemon's working directory (fixed when it \
             was first started), not yours, so it would serve the wrong project.",
            requested.display()
        ));
    }
    Ok(std::fs::canonicalize(requested).unwrap_or_else(|_| requested.to_path_buf()))
}

#[cfg(test)]
mod target_project_tests {
    use super::resolve_target_project;
    use std::path::{Path, PathBuf};

    #[test]
    fn empty_request_falls_back_to_daemon_default() {
        let default = PathBuf::from("/tmp/daemon-default");
        let got = resolve_target_project(Path::new(""), &default).unwrap();
        assert_eq!(got, default);
    }

    #[test]
    fn relative_request_is_rejected() {
        let default = PathBuf::from("/tmp/daemon-default");
        // The bare "." that the buggy client used to send.
        let err = resolve_target_project(Path::new("."), &default).unwrap_err();
        assert!(
            err.contains("absolute"),
            "error must explain the absolute-path requirement: {err}"
        );
        // A nested relative path is rejected too.
        assert!(resolve_target_project(Path::new("foo/bar"), &default).is_err());
    }

    #[test]
    fn absolute_request_is_accepted_and_absolute() {
        // A real directory so canonicalize succeeds; the daemon default is
        // intentionally unrelated to prove the request path wins.
        let tmp = std::env::temp_dir();
        let got = resolve_target_project(&tmp, Path::new("/unused-default")).unwrap();
        assert!(got.is_absolute());
    }
}

#[cfg(test)]
mod loopback_tests {
    use super::is_loopback_bind;

    #[test]
    fn accepts_loopback_encodings() {
        assert!(is_loopback_bind("127.0.0.1:27787"));
        assert!(is_loopback_bind("localhost:9120"));
        assert!(is_loopback_bind("[::1]:27787"));
        // Other 127.x.x.x addresses also map to loopback on unix.
        assert!(is_loopback_bind("127.5.6.7:8080"));
    }

    #[test]
    fn rejects_public_bind_addresses() {
        assert!(!is_loopback_bind("0.0.0.0:27787"));
        assert!(!is_loopback_bind("192.168.1.5:27787"));
        assert!(!is_loopback_bind("10.0.0.1:9120"));
        assert!(!is_loopback_bind("example.com:80"));
    }
}

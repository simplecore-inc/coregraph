//! D1 spike: minimal named-pipe server for manual cross-platform
//! validation of the VSCode extension's IPC transport.
//!
//! Behaviour: open a local-socket listener at the platform-appropriate
//! path, wait for a single client, read one line, reply with "pong\n",
//! close. Exits 0 on success, 1 on failure.
//!
//! Run:
//!   - On Windows: `cargo run --example spike_pipe` then
//!     `node vscode-extension/scripts/windows-pipe-spike.js`.
//!   - On Unix: prints a skip message and exits 0 — the pipe format
//!     (`\\.\pipe\...`) is Windows-specific, and this spike is not
//!     meaningful on POSIX. The Unix transport is already covered by
//!     existing `coregraph server` tests.
//!
//! The real production transport is in `crates/cli/src/ipc.rs`. This
//! file is only a minimal reproducer for the pipe-compatibility
//! question and has no dependency on the rest of the CLI crate.
//!
//! NOTE: This spike requires D2 to land first (which adds `interprocess`
//! to the CLI crate's dev-dependencies). On Windows, if `interprocess`
//! is not yet available as a dep, the program emits a clear error and
//! exits 1. On Unix, it always exits 0 (skip).

fn main() -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        eprintln!(
            "D1 spike is Windows-only: the named-pipe format \\\\.\\pipe\\... \
             is not reachable on this platform. Skipping."
        );
        Ok(())
    }

    #[cfg(windows)]
    {
        // `interprocess` is required for Windows named-pipe server support.
        // That crate is added to [dev-dependencies] in Task D2. Until D2
        // lands, this path emits a clear dependency-gap message and exits 1
        // rather than panicking or silently doing nothing.
        //
        // Once D2 adds `interprocess`, replace this block with:
        //
        //   use std::io::{BufRead, BufReader, Write};
        //   use interprocess::local_socket::{LocalSocketListener, LocalSocketStream};
        //   let path = r"\\.\pipe\coregraph-spike";
        //   eprintln!("spike_pipe: listening on {path}");
        //   let listener = LocalSocketListener::bind(path)?;
        //   let mut conn: LocalSocketStream = listener.accept()?;
        //   let mut reader = BufReader::new(&conn);
        //   let mut line = String::new();
        //   reader.read_line(&mut line)?;
        //   if line.trim() == "ping" {
        //       conn.write_all(b"pong\n")?;
        //       conn.flush()?;
        //   } else {
        //       eprintln!("unexpected input: {line:?}");
        //       std::process::exit(1);
        //   }

        eprintln!(
            "spike_pipe: needs D2's `interprocess` crate to open a Windows \
             named-pipe server. Add `interprocess` to [dev-dependencies] in \
             crates/cli/Cargo.toml (Task D2) and update this example to use \
             interprocess::local_socket::LocalSocketListener."
        );
        std::process::exit(1);
    }
}

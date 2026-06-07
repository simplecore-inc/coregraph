# Daemon Management

The CoreGraph daemon is a background process that maintains the call graph for your project. You can check its status and control it at any time from the Command Palette (`Ctrl+Shift+P` / `Cmd+Shift+P`).

## Commands

| Command | What it does |
|---|---|
| **CoreGraph: Show Daemon Status** | Displays the daemon version, total indexed symbol count, and active project path. Also appends a timestamped entry to the CoreGraph Output channel. |
| **CoreGraph: Restart Daemon** | Stops the running daemon and lets the extension spawn a fresh one on the next IPC request. Use this after updating the `coregraph` binary or if the graph appears stale. A confirmation dialog appears before the restart. |
| **CoreGraph: Stop Daemon** | Shuts down the daemon. CoreGraph features will be unavailable until the next file save triggers an auto-restart. A confirmation dialog appears before the stop. |
| **CoreGraph: Show Logs** | Opens the CoreGraph Output channel — useful for diagnosing connection issues. |

## Auto-restart

The daemon starts automatically when you open a supported file, or on the first save after a manual stop. You do not need to start it manually under normal operation.

## Troubleshooting

1. Run **CoreGraph: Show Daemon Status** — if it fails, the daemon is not running.
2. Check the **CoreGraph** Output channel (`CoreGraph: Show Logs`) for error messages.
3. Verify `coregraph` is on your `$PATH` (`which coregraph`) or set `coregraph.binaryPath` in Settings.
4. Run **CoreGraph: Restart Daemon** to recover from a stuck state.

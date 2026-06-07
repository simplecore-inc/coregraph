# CoreGraph for VS Code

> ⚠ **Status: in development — not yet tested.** This extension is a
> work in progress and has not been verified end to end. Expect rough edges,
> incomplete behavior, and breaking changes; it is not recommended for daily use
> yet. The features and settings below describe the intended surface, not a
> validated release.

Cross-language call-graph intelligence backed by the [`coregraph`](../README.md) CLI. CoreGraph indexes your codebase into a live symbol graph and surfaces reach, impact, and confidence scores directly in the editor — no cloud, no telemetry. The extension is a thin transport: it spawns whatever `coregraph` is on your `$PATH` (via the `coregraph lsp` stdio bridge plus a local IPC socket to the daemon) and renders the results. All intelligence lives in the CLI.

## Quick start

1. Install the CLI so `coregraph` is on your `$PATH`:
   ```bash
   npm install -g @coregraph/cli
   ```
2. Open a workspace folder containing one of the supported languages (see below). When you open a supported file, the extension auto-starts the background daemon and indexes the project; saving a file afterward triggers an incremental reindex. CodeLens annotations appear above your symbols once indexing finishes.
3. On first activation, CoreGraph offers to enable commit-time impact warnings. You can change this later under `coregraph.warnOnCommit.enabled`.

**Supported languages:** Rust, Java, TypeScript, TypeScript React, JavaScript, JavaScript React, Python, Go, Kotlin. The extension activates only when you open a file in one of these.

## Features

| Surface | What you see |
|---|---|
| **CodeLens** | `reach N · impact N.N` annotation above supported declarations (functions, methods, classes, interfaces, structs, enums, constructors). |
| **Hover** | Top callers/callees, stale flag, and confidence for the symbol under the cursor. |
| **Status Bar** | Cursor-tracking summary of the current symbol's impact. Click to focus the CoreGraph Explorer panel. |
| **Diagnostics** | Orphaned symbols and cross-graph inconsistencies surfaced in the Problems panel. |
| **Tree View** | "CoreGraph" panel in the Explorer sidebar — symbols and their dependency edges for the active file. Use the inline **Go to Symbol** action on an edge to jump to its target. |
| **Explorer / SCM badges** | File-decoration badges (`•` / `!` / `‼`) on changed files, colored by the file's confidence-weighted diff impact relative to `HEAD`. |
| **Gutter markers** | Per-line gutter decorations on changed lines, with a hover listing the symbols and impact at that line. |
| **Commit warning** | Optional status-bar warning when the working-tree diff's impact or introduced inconsistencies exceed your thresholds (off by default). |
| **Diff Impact / Review Preview** | WebView panels: per-file impact change relative to `HEAD`, and a Markdown review comment for the current diff. |

Decorations and diagnostics refresh automatically when you save a supported file or when `HEAD` changes.

## Scores glossary

| Score | Meaning |
|---|---|
| **reach** | Distinct symbols reachable from this one in either direction (its callers and its callees), excluding the symbol itself. |
| **edges** | Direct graph edges touching this symbol (callers + callees). |
| **impact** | Sum of edge confidence weights across all traversed edges (both directions); higher means the symbol is more central to the graph. |
| **confidence** | How strongly the call graph trusts the resolved relationships (0–100%). |
| **stale** | Symbol data comes from an older graph snapshot — save the file to refresh. |
| **Orphan** | Symbol has no incoming or outgoing edges — a dead-code candidate. |

## Commands

All commands are under the **CoreGraph** category in the Command Palette.

| Command | Description |
|---|---|
| `CoreGraph: Show Diff Impact` | Open the Diff Impact WebView panel. |
| `CoreGraph: Preview Review Comment` | Open the Review Preview WebView panel. |
| `CoreGraph: Show Daemon Status` | Show daemon version, indexed symbol count, and project path. |
| `CoreGraph: Restart Daemon` | Stop and restart the background daemon (modal confirmation). |
| `CoreGraph: Stop Daemon` | Shut down the daemon (modal confirmation). |
| `CoreGraph: Go to Symbol` | Inline Tree View action — navigate to an edge's target symbol. |
| `CoreGraph: Show Logs` | Open the CoreGraph Output channel. |
| `CoreGraph: Open Walkthrough` | Open the Get Started walkthrough. |

## Settings

| Setting | Default | Description |
|---|---|---|
| `coregraph.binaryPath` | `"coregraph"` | Path to the CLI binary. Override when `coregraph` is not on `$PATH`. |
| `coregraph.trace.server` | `"off"` | LSP message tracing verbosity in the Output panel: `off`, `messages`, or `verbose`. |
| `coregraph.warnOnCommit.enabled` | `false` | Show a status-bar warning when the working-tree diff's impact or introduced inconsistencies exceed thresholds. |
| `coregraph.warnOnCommit.impactThreshold` | `20` | Warn when the confidence-weighted impact of the working-tree diff exceeds this value. |
| `coregraph.warnOnCommit.inconsistencyCount` | `1` | Warn when this many or more inconsistencies are introduced by the diff. |
| `coregraph.diagnostics.excludeTests` | `true` | Exclude symbols under test paths (e.g. `tests/`, `__tests__/`, `*_test.rs`) from orphan and inconsistency diagnostics. Disable to see fixture-level inconsistencies while debugging. |

## How it works

The extension talks to the CLI two ways:

- It starts a `LanguageClient` that spawns `coregraph lsp` for go-to-definition, find-references, and workspace-symbol lookups (used by **Go to Symbol** and other navigation).
- It connects to the daemon's local IPC socket for graph queries (CodeLens, hover, diagnostics, decorations, diff impact). If the daemon is not running, it auto-spawns `coregraph server start --foreground`.

There is no in-extension graph engine. Whatever `coregraph` binary is on your `$PATH` determines the results, so keep the CLI and extension versions in step.

## Install from source

1. Install the CLI: `npm install -g @coregraph/cli` (or build from source with `cargo install --path crates/cli` from the repo root, or use a release binary).
2. Build the extension package:
   ```bash
   cd vscode-extension
   npm install
   npm run compile
   npx vsce package
   ```
3. Install the resulting `.vsix`:
   ```bash
   code --install-extension coregraph-vscode-0.3.2.vsix
   ```

The daemon starts automatically on first use of a supported file. License: MIT.

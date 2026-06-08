# Changelog

User-visible changes per release. Versions follow semver; breaking
changes bump the minor (until 1.0).

## [Unreleased]

## [0.1.0] - 2026-06-08

First public release of the `coregraph` CLI.

### Added

- **Cross-file, multi-language symbol graph** built in a single `index` pass —
  tree-sitter extracts symbols (functions, methods, structs, classes, enums,
  config keys, doc comments) and stack-graphs resolves names across files. No
  language server, build system, or compiler toolchain required. Languages:
  Java, TypeScript, JavaScript, Python, Go, Rust.
- **Confidence-tagged edges** — every edge carries a confidence score (0.0–1.0),
  the origin that produced it (name-resolved vs. syntax-matched vs.
  pattern-matched), and a trust model, filterable with `--min-confidence`.
- **Analysis commands** — `query` (find symbols), `inspect` (symbol at
  FILE:LINE), `impact` (what breaks if this changes), `orphans` (dead code),
  `diff` (impact of a git diff), `inconsistencies` (cross-enum / api-path /
  config-key mismatches), `stats`, and `review` (auto-comment a GitHub PR with
  the diff impact summary).
- **Background daemon** serving the graph over an IPC socket, with
  `server` lifecycle management (start/stop/status/restart/install/uninstall)
  and `--no-auto-start` for in-process fallback.
- **Agent & editor bridges** — `mcp` (MCP stdio bridge for LLM agents),
  `lsp` (LSP stdio bridge for editors), and an optional HTTP API.
- **Export & persistence** — `export` to dot / cypher / json-graph,
  `snapshot` save/load of a binary graph, and `watch` for incremental rebuilds
  on file changes.
- **Batch & config** — `batch` runs multiple queries from a JSON file;
  `config` shows, sets, or initializes configuration; `plugin` manages hooks.
- **Output controls** — `human` / `llm` / `json` output formats, token-budget
  capping, hop-limit traversal control, and `--fast` / `--standard` / `--full`
  analysis presets.
- **Distribution** — published to npm as `@coregraph/cli` with per-platform
  binaries for macOS (arm64, x64), Linux (x64, arm64; musl-static), and
  Windows (x64, arm64); the same prebuilt binaries are attached to each GitHub
  Release as `coregraph-<version>-<os>-<cpu>.{tar.gz,zip}` with `SHA256SUMS`.
  MIT licensed.

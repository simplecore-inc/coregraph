# Changelog

User-visible changes per release. Versions follow semver; breaking
changes bump the minor (until 1.0).

## [Unreleased]

## [0.1.1] - 2026-06-09

### Added

- **Multi-agent integration kit** (`agents/`) — a Claude Code plugin and
  marketplace (`/plugin marketplace add simplecore-inc/coregraph`), an `AGENTS.md`
  for Codex / Gemini CLI / opencode, and a guidance skill that prefers the symbol
  graph over a raw grep/read sweep for structural questions.
- **MCP `impact` `transitive` flag** — pass `transitive: true` to get the
  transitive closure up to `depth`; the default stays direct (depth-1) dependents.
  (Previously the advertised `depth` was inert over MCP.)

### Fixed

- **MCP tool descriptions** corrected: `inconsistencies` covers enum / api-path /
  config-key (doc-drift is CLI-only); `stats` reports symbol and edge counts;
  `orphans` is described as dead-code candidates.
- **`--min-confidence` help** now warns that `NameResolved` `calls` edges sit at
  ~0.85, so `0.90` drops them and yields an empty caller set — keep `≤ 0.85`.

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

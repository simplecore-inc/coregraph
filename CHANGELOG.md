# Changelog

User-visible changes per release. Versions follow semver; breaking
changes bump the minor (until 1.0).

## [Unreleased]

## [0.2.0] - 2026-06-10

### Added

- **`coregraph viz` — the atlas viewer.** Serves an interactive 3D view of the
  symbol graph on `127.0.0.1:7321` (per-process token guard), fed directly from
  daemon memory: project picker with daemon auto-start, fuzzy search,
  neighborhood isolate, per-symbol detail with a source preview, analysis
  overlays (impact blast-radius gradient with risk and affected tests, dead
  code, cross-file inconsistency pairs, git-diff impact, shortest path),
  cluster-by-unit with translucent boundary hulls, degree/hub/kind/confidence
  filters, share links that restore the exact view, PNG / json-graph export,
  and a change poll that offers a one-click reload when the daemon's graph
  moves on.
- **Macro-body call extraction (Rust).** Call references inside macro token
  trees (`json!`, `format!`, `assert!`, …) are now recovered lexically;
  tree-sitter's call patterns never fire inside raw token trees, so these
  references were previously missing from the graph.
- Daemon IPC methods `export_graph` (json-graph dump of the in-memory graph)
  and `reload_project` (forced rebuild from source, bypassing the snapshot).

### Changed

- **`impact` now counts transitive dependents only** — the symbols that would
  break if the seed changed (callers, their callers, …), following incoming
  impact-bearing edges. The previous bidirectional walk made `reachable` a
  connectivity count: a depth-5 sweep from a 3-caller helper reached 74% of
  this repo's graph through shared callees. File/doc container nodes are
  excluded from the cone. Expect substantially smaller (and now meaningful)
  `reachable` numbers.
- **`impact` seed disambiguation** — among same-name definitions the seed is
  now the one with the most incoming dependents, instead of the first match
  (which could land on an uncalled twin and report zero impact).

## [0.1.3] - 2026-06-09

### Added

- **Daemon-cached `impact`, `diff`, `inconsistencies`** — these now reuse the
  background daemon's cached graph instead of rebuilding it on every call (like
  `query`/`orphans`/`stats`), so repeat invocations skip the rebuild. On-demand
  healing keeps their results fresh for just-edited files.

### Changed

- **`inconsistencies --output-format json` shape changed** — now
  `{count, reports: [...]}` with kebab-case `category` (`api-path`, not
  `ApiPath`), marker-stripped names (no `api_path::` prefix), and a 0-based
  `line` per node. Previously a flat array. Scripts parsing the old shape must
  update.
- **`impact --output-format json` shape changed** — the top-level key is now
  `symbol` (was `seed`), and it includes the full `nodes[]` list and the
  complete 4-factor risk object.
- **In-process output paths are now canonical-absolute**, matching the daemon.
  `coregraph impact foo` from a project directory prints absolute paths whether
  or not the daemon is running (in-process previously printed relative `./x`).

### Fixed

- Daemon-served `impact` no longer shrinks the default reachable set — it now
  honors the requested `--hop-limit` instead of a hardcoded depth of 1 for
  non-transitive queries.
- `impact`/`diff`/`inconsistencies` produce identical output whether served
  in-process or by the daemon.

## [0.1.2] - 2026-06-09

### Added

- **Analysis-surface exclude** (`[analysis].exclude`) — list generated / noise
  files here to keep them indexed (their edges still connect the symbols they
  reference) while hiding their own symbols from dead-code (`orphans`) reports.
  Distinct from `[index].exclude`, which drops a file's nodes *and* edges and can
  turn a symbol referenced only by an excluded file into a false orphan.
- **Plugin install without cloning** the source repo — add the catalog by raw
  URL (`/plugin marketplace add https://raw.githubusercontent.com/simplecore-inc/coregraph/main/.claude-plugin/marketplace.json`),
  then `/plugin install coregraph@coregraph` sparsely fetches only
  `agents/coregraph`. The owner/repo shorthand
  (`/plugin marketplace add simplecore-inc/coregraph`) also works but git-clones
  the whole source repo to read the catalog; the subsequent install stays sparse
  either way.

### Fixed

- **TS value-position references** are now captured: a module-level const used
  only via subscript (`OBJ[key]`), member access (`obj.x` / `set.has(x)`), or a
  JSX prop (`prop={CONST}`) is no longer reported as dead code.
- **Aliased named imports** resolve per specifier, so same-named imports — e.g. a
  generated TanStack `routeTree` importing `Route` from every route file under a
  distinct alias — connect their targets instead of being falsely orphaned.
- **Config edits invalidate the daemon snapshot**: changing
  `.coregraph/config.toml` (e.g. an `exclude` list) now rebuilds the graph instead
  of warm-loading a stale, pre-edit snapshot.
- **Daemon IPC requests are routed by absolute project path**, so a client sending
  a relative path no longer gets the wrong project served.

### Changed

- `orphans --help` documents the detector's scope: it reports only
  fully-disconnected symbols, so a clean result is triage, not a census.

## [0.1.1] - 2026-06-09

### Added

- **Multi-agent integration kit** (`agents/`) — a Claude Code plugin and
  marketplace (`/plugin marketplace add simplecore-inc/coregraph`), an `AGENTS.md`
  for Codex / Gemini CLI / opencode, and a guidance skill that prefers the symbol
  graph over a raw grep/read sweep for structural questions.
- **MCP `impact` `transitive` flag** — pass `transitive: true` to get the
  transitive closure up to `depth`. (Previously the advertised `depth` was inert
  over MCP.) The default then returned direct (depth-1) dependents; as of 0.1.3
  the traversal always honors `depth` (default 5) and `transitive` is render-only
  metadata, so a default MCP call returns the depth-bounded transitive closure.

### Fixed

- **MCP tool descriptions** corrected: `inconsistencies` covers enum / api-path /
  config-key (doc-drift is not advertised over MCP, though the shared handler
  still runs it when a client passes `category: "doc-drift"`); `stats` reports
  symbol and edge counts; `orphans` is described as dead-code candidates.
- **`--min-confidence` help** now warns that `NameResolved` `calls` edges sit at
  ~0.85, so `0.90` drops them and yields an empty caller set — keep `≤ 0.85`.

## [0.1.0] - 2026-06-08

First public release of the `coregraph` CLI.

### Added

- **Cross-file, multi-language symbol graph** built in a single `index` pass —
  tree-sitter extracts symbols (functions, methods, structs, classes, enums,
  config keys, doc comments) and stack-graphs resolves names across files. No
  language server, build system, or compiler toolchain required. Languages:
  Java, Kotlin, TypeScript, JavaScript, Python, Go, Rust.
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

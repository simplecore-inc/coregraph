# Changelog

User-visible changes per release. Versions follow semver; breaking
changes bump the minor (until 1.0).

## [Unreleased]

## [0.2.9] - 2026-06-19

### Fixed

- **Atlas edge total now matches the project picker.** Since the bridge omits
  `Resolves` from the graph payload, the loaded atlas showed only the shipped
  edge subset (e.g. ~93k) while the picker showed the daemon's full `edge_count`
  (e.g. 341,776) — the same project appeared to have two different sizes. The
  json-graph export now carries `total_nodes`/`total_edges` (the unfiltered
  graph size) and the atlas uses them for the project total in the header,
  footer and load toast, so the number is consistent everywhere while the
  payload stays small. Exports without the fields fall back to the loaded counts.

## [0.2.8] - 2026-06-19

### Fixed

- **Incremental rebuilds no longer diverge from a clean build.** After 0.2.7
  stopped the synthetic-`Module` ratchet, the watcher's `build_graph_incremental`
  still inflated a daemon's graph by ~6% per rebuild because two derive stages
  were not idempotent when re-run on an already-built graph: `structural_pass`
  wrapped nodes added by later passes (`DocComment`/`DocSection` and synthetic
  `Module` nodes) in `Contains`/`BelongsTo`, and `resolve_references`
  re-resolved `Calls`/`References` onto the `ExternalPackage` stubs a previous
  run had minted. Both now exclude those nodes (containers and doc-layer nodes
  are not containment children; `ExternalPackage` stubs are not resolution
  candidates and are reused from the cache), and the Java/Kotlin `Inherits`
  derivation moved into its own stage after `resolve_references` so it covers
  the resolver's cross-file `Extends` identically on every run. Re-running the
  whole derive pipeline on an unchanged graph is now a no-op, so a long-lived
  daemon stays at its clean edge count instead of ballooning over time.

## [0.2.7] - 2026-06-19

### Fixed

- **Daemon no longer ratchets its edge count upward over its lifetime.** The
  watcher's incremental-rebuild path (`build_graph_incremental`) re-runs
  `structural_pass` on the already-structured cached graph. That pass reused
  `File` nodes by path but allocated a fresh synthetic `Module` node on every
  call, so each rebuild re-emitted a `BelongsTo` edge for every symbol while the
  old edges survived (their symbols are not invalidated). The containment layer
  therefore duplicated on each rebuild and the result was persisted to the
  snapshot, so the graph grew further on every restart — a long-lived daemon
  reached roughly 3x its clean edge count on a large Java project (e.g. ~917k vs
  the true ~341k edges, with `BelongsTo` alone at ~6.5x). `structural_pass` now
  reuses existing `Module` nodes by name, making it idempotent like the `File`
  pass; repeated rebuilds hold steady. This is the edge-level counterpart to the
  0.2.6 fix for symbol-level heal duplication.

### Added

- **Atlas (`coregraph viz`) omits `Resolves` edges from the graph payload.**
  Name-resolution edges are the bulk of a large graph and are already hidden
  from the rendered view, so shipping them only inflated the browser's parse,
  memory and filter cost. `export_graph` gains an optional `exclude_edge_kinds`
  parameter (absent = full graph, so the CLI `export --format json-graph` is
  unchanged); the viewer bridges default it to `["Resolves"]`, overridable via
  `COREGRAPH_VIZ_EXCLUDE_EDGE_KINDS`. Cuts the viz payload roughly 3x.

## [0.2.6] - 2026-06-16

### Fixed

- **Daemon no longer inflates symbol and orphan counts over its lifetime.** The
  on-demand heal path re-extracted changed files with a bare `extractor.extract`
  that never removed the file's existing nodes; because `insert_node` always
  allocates a fresh id, every heal duplicated the file's symbols, accumulating
  edge-less phantom orphans. A daemon-served graph therefore drifted away from a
  clean in-process build the longer it ran (e.g. `orphans` reporting 933 vs the
  true 177 on a 921-file Spring monorepo). The heal loop now shares the surgical
  single-file reindex used by the `reindex` IPC fast-path
  (`reindex_file_in_place_core`: remove the file's old nodes, re-extract, re-link
  cross-file edges), which is idempotent on an unchanged file — so repeated heals
  leave the graph's symbol and edge counts stable.

## [0.2.5] - 2026-06-16

### Performance

- **Doc-comment indexing no longer recompiles its tree-sitter query per file.**
  `extract_block_doc_comments` compiled its per-language (static) query via
  `Query::new` on every file — thousands of redundant compilations on a large
  monorepo (a profiled hot spot after the 0.2.4 resolver fix). The compiled
  query is now cached per thread (keyed by the query string, 1:1 with the
  language across all callers), shaving ~0.5s off a ~900-file index and more on
  doc-comment-heavy trees. Behaviour is unchanged.

## [0.2.4] - 2026-06-16

### Fixed

- **Release builds no longer index large projects pathologically slowly.** The
  stack-graphs resolver's shadow-suppression step compared every stitched
  partial path against every other (O(n²)). Because path stitching is
  wall-clock bounded, an optimized (release) build finds far more paths than a
  debug build in the same time budget, so the O(n²) post-pass exploded —
  release indexing of a ~1k-file monorepo took **8m46s vs 26s for an
  unoptimized debug build**, with runaway memory. Paths from different
  references can never shadow each other, so the comparison is now grouped by
  reference and runs only within each (tiny) group, restoring effectively
  linear behaviour (**~10s** on the same project). Affected every prior release
  on sufficiently large/interconnected codebases.

## [0.2.3] - 2026-06-16

### Fixed

- **Daemon serves requests during its initial index.** The daemon bound its IPC
  socket and then indexed the default project synchronously *before* entering
  the accept loop, so a client connecting mid-index (e.g. the `viz` atlas
  bridge probing `status`) was accepted into the listen backlog but never
  answered until indexing finished — its receive timeout surfaced as a cryptic
  `Resource temporarily unavailable (os error 35)` "bridge error". The default
  project now preloads on a background thread and the accept loop starts
  immediately. IPC receive timeouts also report an actionable message
  ("daemon did not reply … it may still be indexing") instead of the raw OS
  errno.
- **Single daemon instance.** Several `server start`s firing at once (multiple
  MCP/LSP/viz bridges auto-spawning the daemon on a fresh boot) could race
  `remove_file` + `bind`: losers crashed with `Address already in use`, and a
  late binder could orphan a second live daemon that held a graph in memory but
  owned no socket. A process-lifetime advisory lock now guarantees exactly one
  daemon; losers exit cleanly.

### Changed

- **Indexing always honors `.gitignore` and the default exclude set.** The index
  walk now applies the universal build-output / dependency excludes (`build/`,
  `node_modules/`, `.gradle/`, `target/`, `dist/`, `out/`, …) it already shared
  with analysis — previously these were skipped only by `.gitignore`, so a
  project without a (complete) `.gitignore` indexed and parsed thousands of
  generated/vendored files. `.gitignore` is now honored even outside a git
  repository, and excluded directories are pruned at the walk level so large
  `build/` and `node_modules/` trees are never descended into.

## [0.2.2] - 2026-06-12

### Added

- **`coregraph viz --detach` / `--stop`.** `--detach` runs the atlas server in
  the background, detached from the terminal session (it survives the terminal
  closing), and returns once the port answers; output goes to `viz.log` in the
  daemon runtime directory and the instance is recorded in `viz.pid`. It
  refuses to start when the port is already serving. `--stop` terminates the
  recorded instance and cleans up; a record whose port no longer answers is
  treated as stale and removed without signaling anything.

## [0.2.1] - 2026-06-12

### Added

- **`coregraph config recommend [--write]`.** Derives suggested `config.toml`
  tuning from the indexed graph through four measured signals: `[index].exclude`
  candidates (files dominated by data-kind symbols), a `string_match_max_files`
  cap picked from a projection over candidate caps, `[inconsistencies]
  disable = ["api-path"]` when every near-miss pair is same-language-family
  (a frontend-only repo has no cross-language contract to drift), and
  `[analysis].exclude` candidates for generated files detected by content
  markers (`@generated`, `DO NOT EDIT`, `Code generated by`). Prints a
  paste-ready TOML block with per-recommendation reasons (human/json/llm);
  `--write` applies it with a comment-preserving merge that aborts before
  writing anything when an existing value has an incompatible shape. Also a
  daemon method and a sixth MCP tool (`recommend`).
- Config keys `[index] string_match_max_files` (default 8, `0` = unlimited)
  and `[inconsistencies] disable` / `api_path_min_segments` (default 2).
- **Config lint.** `coregraph index` (stderr) and `config show` (inline ⚠)
  now warn about TOML parse failures, unknown sections, and unknown keys in
  `.coregraph/config.toml` instead of silently ignoring them.
- **Post-index noise note.** Human-format `index` output suggests
  `[index].exclude` candidates when a few data-dominated files (config keys /
  string literals / doc sections) hold an outsized share of all symbols.

### Changed

- **`stats --breakdown` is monorepo-aware.** Per-package counts come from the
  project's own manifests (Cargo workspaces, npm/pnpm workspaces, Maven,
  Gradle, …) instead of assuming a `crates/` directory layout; the
  most-referenced table now counts only impact-bearing edges between real
  symbols (file/doc containers excluded), so structural edges no longer bury
  the actual hubs; synthetic external-package nodes are shown as a labeled
  `(external)` row.
- **Cross-file StringMatch pairing is capped per string value**
  (`string_match_max_files`, default 8): a literal occurring in more distinct
  files is treated as a convention string and no longer emits O(k²) edges.
  Measured on a real React monorepo this removed roughly half of all
  ApiPathMatch edges (previously 27% of the graph).
- **api-path near-miss reports require two path segments** on both sides by
  default (`api_path_min_segments`); single-segment pairs like `/auth` vs
  `/auths` are route-name conventions, not typos.
- **doc-drift is rename-aware.** A single-underscore unused-marker rename
  (`name` documented, `_name` in the signature) is no longer reported;
  destructured shorthand parameters (`{ a, b }`) are recognized; genuine
  mismatches now carry closest-signature rename candidates in the detail line
  and a `candidates` array in the JSON output.

### Fixed

- A malformed glob in `[index].exclude` no longer silently disables every
  exclude pattern (built-in defaults included); the bad pattern is skipped
  with a stderr warning and everything else keeps applying.
- JSX component usage across npm-workspace packages now produces symbol-level
  Calls edges: bare import specifiers (`@scope/pkg`, `@scope/pkg/subpath`)
  resolve through the workspace's own manifest (package `exports`/`module`/
  `main`, then `src/index.*`), so cross-package component hubs are visible to
  `impact` instead of being dropped to the file-level fallback.
- `stats --breakdown` no longer prints a nameless top-files row for synthetic
  external-package nodes.

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

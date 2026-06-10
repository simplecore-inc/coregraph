# Tech Stack and Per-Language Parsing Strategy

CoreGraph is a single Rust workspace (binary: `coregraph`, version `0.1.3`, MIT).
It turns a multi-language / monorepo codebase into one queryable symbol graph by
layering two parsing technologies — **tree-sitter** for fast symbol extraction and
**stack-graphs** for cross-file name resolution — then serving the result from a
background daemon over an IPC socket and (optionally) HTTP.

There is no SCIP and no external indexer. Everything below ships in this repo.

---

## At a glance

```
manifest  →  tree-sitter  →  stack-graphs  →  symbol graph  →  daemon / queries
(modules)    (symbols)       (resolution)     (petgraph)        (IPC, HTTP, LSP, MCP)
```

| Layer | Crate | What it does |
|---|---|---|
| Manifest | `coregraph-manifest` | Detect project structure: Cargo / npm-pnpm-yarn / Gradle / Maven / Go / Python / Vite. Module boundaries, workspace layout, internal vs external deps. |
| Symbol extraction | `coregraph-extractor` | tree-sitter parse per file; emit definition/reference symbols, config keys, string literals, doc comments. |
| Name resolution | `coregraph-stack` | stack-graphs cross-file binding; produce `Resolves` edges. |
| Graph | `coregraph-graph` | The `SymbolGraph` itself (petgraph), confidence/trust model, bincode snapshots, cross-language mediators. |
| Core types | `coregraph-core` | Shared `SymbolNode` / `SymbolKind` / `EdgeKind` / config types — no OS or IO deps. |
| Queries | `coregraph-query` | Impact, orphans, inconsistencies, risk scoring. |
| File watching | `coregraph-watcher` | `notify`-based incremental rebuild. |
| HTTP server | `coregraph-server` | axum routes for the daemon's HTTP mode. |
| CLI + daemon | `coregraph` (cli) | clap commands, IPC client/daemon, LSP and MCP stdio bridges. |

---

## Workspace crates

The workspace has nine members. The CLI binary (`coregraph`) depends on the rest.

| Crate | Role |
|---|---|
| `coregraph-core` | Shared types and config. No IO; pure data + hashing. |
| `coregraph-manifest` | Build-system / package-manager manifest parsing. |
| `coregraph-extractor` | tree-sitter symbol extraction (code, config, markdown). |
| `coregraph-stack` | stack-graphs name resolution. |
| `coregraph-graph` | `SymbolGraph` storage, confidence, snapshots, mediators. |
| `coregraph-query` | Graph queries and analysis. |
| `coregraph-watcher` | File-change watcher. |
| `coregraph-server` | axum HTTP API. |
| `coregraph` (cli) | Command-line entrypoint + background daemon. |

---

## Key dependencies

These are the third-party crates that shape behaviour. Versions are pinned in the
workspace `Cargo.toml`.

| Role | Crate(s) | Notes |
|---|---|---|
| Incremental parsing | `tree-sitter` | One grammar per language; partial re-parse on edit. |
| Language grammars | `tree-sitter-{java,typescript,javascript,python,go,rust}`, `tree-sitter-kotlin-ng` | Kotlin uses the community `tree-sitter-kotlin-ng` grammar (ABI-compatible with tree-sitter 0.24). |
| Name resolution | `tree-sitter-stack-graphs`, `stack-graphs`, `lsp-positions` | Library-level `StackGraph` used directly — no SQLite, no CLI. |
| Upstream resolution rules | `tree-sitter-stack-graphs-{java,typescript,javascript,python}` | Prebuilt stack-graphs rules for those four languages. |
| Graph structure | `petgraph` | Directed graph, traversal, transitive reachability. |
| Membership filter | `bloomfilter` | Fast file→symbol membership checks. |
| Content hashing | `xxhash-rust` (xxh3) | Change detection / content hashes. |
| Snapshots | `bincode`, `serde` | Binary graph snapshots (schema v6). |
| File watching | `notify` | inotify / FSEvents / ReadDirectoryChanges. |
| HTTP server | `axum`, `tokio`, `tower` | The daemon's HTTP mode. |
| IPC | `interprocess` | Thin-client ↔ daemon socket. |
| CLI | `clap` (derive) | All subcommands and flags. |
| Config parsing | `serde_yaml`, `toml`, `serde_json` | Config-file key paths and project config. |
| XML parsing | `quick-xml` | Maven `pom.xml`. |
| `.gitignore` parsing | `ignore` | ripgrep's parser; skip build output and vendored dirs. |
| File-content cache | `lru` | Bounded LRU cache of raw source-file contents for LSP/MCP range resolution. |
| Parallelism | `rayon` | Data-parallel file parsing and path stitching. |
| Config paths | `dirs` | Locate the global config directory. |
| Benchmarks | `criterion` | `coregraph-extractor` ships a `build_graph` bench. |

Notes worth calling out:

- **The LSP and MCP bridges are hand-rolled** over stdio with `serde_json` JSON-RPC.
  There is no `tower-lsp` dependency.
- **Config files are parsed without tree-sitter.** YAML/`yml`, TOML, and JSON keys
  are walked with `serde_yaml` / `toml` / `serde_json` to produce `ConfigKey` nodes,
  while `.properties` keys are read by a small hand-rolled line parser
  (`key=value` / `key:value`, with `#` / `!` comments) in the same
  `ConfigExtractor` — also emitting `ConfigKey` nodes. Markdown is handled by the
  extractor's own `markdown` / `doc_comment` modules, not a markdown crate.
- The release profile uses fat LTO with a single codegen unit and stripped symbols
  — the CLI is the deliverable, so the build favours a smaller, faster binary.

---

## Parsing pipeline

### 1. Manifest — project structure

`coregraph-manifest` reads the build-system manifests to learn module boundaries,
the workspace layout, and which dependencies are internal vs external. Supported
manifests: Cargo, npm / pnpm / yarn, Gradle, Maven, Go modules, Python
(`pyproject.toml` / `requirements`), and Vite. Build output and generated files are
excluded up front (gitignore rules via `ignore`, plus minified/generated detection
— for example, the indexer reports `skipped 1 minified/generated file(s)`).

### 2. tree-sitter — symbol extraction

Every source file is parsed with its tree-sitter grammar. The extractor emits
definition and reference symbols, config keys, string literals, and doc comments.
Because tree-sitter is incremental, an edited file is re-parsed in part rather than
from scratch — this is what makes the `watch` command and the daemon's incremental
reindex cheap.

This layer alone gives `SyntaxMatched` edges (confidence ~0.85): structurally
correct, but not cross-file-resolved.

### 3. stack-graphs — cross-file name resolution

For precise cross-file binding, CoreGraph uses the **library-level** `StackGraph`
type directly. (The `tree-sitter-stack-graphs` CLI persists to SQLite; CoreGraph
does not — it keeps the graph in memory.) The flow:

```rust
// Simplified sketch of the real flow in crates/stack/src/backend.rs.
use stack_graphs::graph::StackGraph;
use stack_graphs::partial::PartialPaths;
use stack_graphs::stitching::{
    Database, DatabaseCandidates, ForwardPartialPathStitcher, StitcherConfig,
};

// 1. Build a StackGraph and add a node graph per file (driven by the .tsg rules).
let mut sg = StackGraph::new();
let file = sg.add_file("src/main.rs").unwrap();

// 2. Compute minimal partial paths for each file and feed them into the Database.
let mut partials = PartialPaths::new();
let mut db = Database::new();
ForwardPartialPathStitcher::find_minimal_partial_path_set_in_file(
    &sg, &mut partials, file, StitcherConfig::default(), &cancellation,
    |g, p, path| db.add_partial_path(g, p, path.clone()),
);

// 3. Stitch complete paths for every reference node to resolve references
//    to definitions, using the Database as the candidate source.
let references: Vec<_> = sg.iter_nodes().filter(|&h| sg[h].is_reference()).collect();
let mut candidates = DatabaseCandidates::new(&sg, &mut partials, &mut db);
ForwardPartialPathStitcher::find_all_complete_partial_paths(
    &mut candidates, references, StitcherConfig::default(), &cancellation,
    |_g, _ps, path| {
        // 4. Map each resolved reference→definition pair to a Resolves edge.
        let reference = graph.find_or_create(path.start_node);
        let definition = graph.find_or_create(path.end_node);
        graph.add_edge(reference, definition, EdgeKind::Resolves);
    },
);
```

Resolved bindings become `Resolves` edges with `NameResolved` origin (confidence
~0.95) — higher trust than syntactic matches.

---

## Language support

Stack-graphs covers **all seven code languages**. Four use upstream rule packages;
three use hand-authored `.tsg` rules that ship in this repo under
`crates/stack/rules/`.

| Language | tree-sitter grammar | stack-graphs rules | Cross-file resolution |
|---|---|---|---|
| Java | `tree-sitter-java` | upstream (`tree-sitter-stack-graphs-java`) | ✓ |
| TypeScript | `tree-sitter-typescript` | upstream (`tree-sitter-stack-graphs-typescript`) | ✓ |
| JavaScript | `tree-sitter-javascript` | upstream (`tree-sitter-stack-graphs-javascript`) | ✓ |
| Python | `tree-sitter-python` | upstream (`tree-sitter-stack-graphs-python`) | ✓ |
| Go | `tree-sitter-go` | hand-authored (`crates/stack/rules/go.tsg`) | ✓ |
| Rust | `tree-sitter-rust` | hand-authored (`crates/stack/rules/rust.tsg`) | ✓ |
| Kotlin | `tree-sitter-kotlin-ng` | hand-authored (`crates/stack/rules/kotlin.tsg`) | ✓ |

The hand-authored rules are layered onto the tree-sitter grammar via
`LanguageConfiguration::from_sources` — there is no upstream stack-graphs package
for Go, Rust, or Kotlin.

When resolution does not produce a binding (or a file is in a language with no
rules at all), CoreGraph falls back to tree-sitter syntactic matching, which still
yields useful — if lower-confidence — edges.

### Config and documentation files

These are parsed for keys and structure, not for code symbols:

| Format | Handled by | Produces |
|---|---|---|
| YAML / `yml`, TOML, JSON | `serde_yaml` / `toml` / `serde_json` | `ConfigKey` nodes (dotted key paths) |
| `.properties` | hand-rolled line parser (`key=value` / `key:value`) | `ConfigKey` nodes |
| Markdown | extractor `markdown` / `doc_comment` modules | `DocSection` / `DocComment` nodes and doc-link edges |
| Maven `pom.xml` | `quick-xml` | dependency / module info |

Config keys link back to code through the cross-language mediators (Spring config,
Docker Compose, and so on), producing `ExternallyMediated` edges such as `Configures`.

---

## Where things live

| Concern | Location |
|---|---|
| Hand-authored stack-graphs rules | `crates/stack/rules/{go,rust,kotlin}.tsg` |
| Graph storage and snapshots | `crates/graph/src/` |
| Cross-language mediators | `crates/graph/src/mediator/` (Spring DI/config, React Router, Docker Compose, Go DI) |
| Config-key extraction | `crates/extractor/src/config_extractor.rs` |
| Markdown / doc extraction | `crates/extractor/src/markdown.rs`, `doc_comment.rs` |
| LSP / MCP stdio bridges | `crates/cli/src/commands/lsp.rs`, `mcp.rs` |
| HTTP routes | `crates/server/src/routes.rs`, `handlers.rs` |

---

For the confidence and trust model see [confidence.md](../confidence.md); for the
full graph model (symbol kinds, edge kinds, origins) see
[graph-model.md](../graph-model.md).

[Back to index](../README.md)

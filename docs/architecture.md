# Architecture

How CoreGraph runs: a background daemon holds an in-memory symbol graph per
project, and thin clients (CLI, IDE via LSP, LLM agent via MCP, scripts via
HTTP) talk to it over a local socket. This page is for users and contributors
who want to understand the runtime without reading the source.

## The short version

```
Clients (thin)            Daemon (one process)            On disk
─────────────────         ──────────────────────────      ─────────────────
coregraph <cmd>  ──IPC──▶  Project Manager (LRU cache)     .coregraph/
coregraph lsp    ──IPC──▶    └─ per-project SymbolGraph       snapshot.bin
coregraph mcp    ──IPC──▶       (petgraph + indexes)          config.toml
HTTP client      ──TCP──▶  Edge evaluator (trust/confidence)
                           File watcher → invalidate → heal
```

1. You run a command such as `coregraph query UserController`.
2. The CLI tries to connect to the daemon's IPC socket. If nothing is
   listening (and auto-start is on), it spawns the daemon in the background
   and waits for the socket to come up, then forwards the request.
3. The daemon loads the project's graph (from a `snapshot.bin` if present,
   otherwise by indexing the source tree), keeps it in memory, and answers.
4. Subsequent commands reuse the in-memory graph, so they return immediately.
5. After a configurable idle period the daemon saves a snapshot, frees the
   graph, and eventually self-terminates. The next command restarts it.

To skip the daemon entirely and build the graph in-process, pass
`--no-auto-start` (or set `COREGRAPH_NO_AUTO_START=1`).

## Crate layout

CoreGraph is a Cargo workspace. The crates, top to bottom:

| Crate | Role |
|---|---|
| `core` | Shared domain types: `SymbolNode`, `DirectEdge`, `SymbolId`, `SymbolKind`, `EdgeKind`, span/file-state types. Pure types, no I/O. |
| `extractor` | tree-sitter symbol extraction per language, plus config/doc/markdown extractors. Holds the `.scm` query files. |
| `stack` | stack-graphs integration: cross-file name resolution. Bundles hand-authored `.tsg` rules for Go, Rust, and Kotlin. |
| `manifest` | Manifest/dependency parsers (npm, Cargo, Gradle, Maven, Go modules, Python, Vite) used to scope packages and detect generated files. |
| `graph` | The in-memory symbol graph engine: the `SymbolGraph` itself, indexes, bloom filters, the edge evaluator (trust + confidence), epoch versioning, invalidation/healing, mediators, snapshot serialization, risk scoring. |
| `query` | Query and serialization: subgraph extraction, token budget, pagination, orphans, inconsistencies, impact, and the human/llm/json output writers. |
| `server` | The HTTP API only (axum): routes and handlers over a shared `SymbolGraph`. |
| `watcher` | File watching, debouncing, and git-aware diff (`watch`, `diff`). |
| `cli` | The binary. The thin-client logic, the daemon process, IPC, the project manager, the LSP/MCP bridges, and all subcommand handlers. |

A note on where things live, because it is easy to assume otherwise: the
**daemon, IPC socket, project manager, and the LSP/MCP bridges all live in the
`cli` crate** (`crates/cli/src/{daemon.rs, ipc.rs, project_manager.rs,
commands/lsp.rs, commands/mcp.rs}`). The `server` crate is *only* the HTTP API.
Symbol extraction lives in `extractor`, not a separate parser crate.

### Where to look

```
crates/
├── core/src/            symbol.rs, edge.rs, span.rs, file_state.rs, graph.rs
├── extractor/src/       {rust,go,java,kotlin,python}_extractor.rs, typescript.rs,
│   │                    javascript.rs, config_extractor.rs, doc_comment.rs,
│   │                    markdown.rs, drift.rs, string_literal_extractor.rs
│   └── queries/         <lang>.scm (symbols) + <lang>-refs.scm (references)
├── stack/src/           resolver.rs, backend.rs
│   └── rules/           go.tsg, rust.tsg, kotlin.tsg  (hand-authored)
├── manifest/src/        npm.rs, cargo_parser.rs, gradle.rs, maven.rs, go.rs,
│                        python.rs, vite.rs, detector.rs, filter.rs
├── graph/src/           symbol_graph.rs, index.rs, bloom.rs, value_index.rs,
│   │                    edge_evaluator.rs, epoch.rs, invalidation.rs,
│   │                    healing.rs, risk.rs, snapshot.rs
│   └── mediator/        spring_di.rs, spring_config.rs, react_router.rs,
│                        docker_compose.rs, go_di.rs
├── query/src/           impact.rs, orphans.rs, inconsistencies.rs, budget.rs,
│   │                    paginate.rs, library.rs
│   └── serialize/       human.rs, llm.rs, json.rs
├── server/src/          routes.rs, handlers.rs, lib.rs   (HTTP only)
├── watcher/src/         lib.rs, debounce.rs, git_diff.rs
└── cli/src/             main.rs, daemon.rs, ipc.rs, dispatch.rs,
    │                    project_manager.rs, graph_loader.rs
    └── commands/        index, query, inspect, stats, orphans, impact, diff,
                         review, inconsistencies, export, snapshot, config,
                         server, lsp, mcp, watch_diff, batch, plugin
```

## The daemon and thin clients

CoreGraph splits into a long-lived **daemon** that owns the graph and **thin
clients** that just send requests. This keeps the expensive part — building the
graph — out of every command's hot path.

### Client startup

When you run a command, the CLI:

1. Tries to connect to the IPC socket. On Unix this is a filesystem socket
   (under the runtime dir, next to the pid-file); on Windows it is a named
   pipe.
2. On success, forwards the request and prints the response.
3. If nothing is listening and auto-start is enabled, it spawns the daemon
   (`coregraph server start --foreground`, detached with `setsid` on Unix /
   its own process group on Windows), polls until the socket is ready, then
   forwards the request.
4. If auto-start is suppressed (`--no-auto-start` or
   `COREGRAPH_NO_AUTO_START=1`), it builds the graph in-process instead. Useful
   for CI, one-off scripts, or debugging.

All clients speak the same small IPC protocol. The daemon dispatches five
public methods — `query`, `impact`, `orphans`, `inconsistencies`, `stats` — to
in-process handlers, plus internal methods for the LSP/MCP bridges (`inspect`,
`impact_batch`, and the LSP definition/references/symbol routes). Both the CLI
and the bridges go through this one dispatch path, so the CLI, IDE, and LLM
agent always see the same graph and the same results.

### The project manager

One daemon serves many projects. The project manager
(`crates/cli/src/project_manager.rs`) keeps each project's graph behind its own
`Arc<RwLock<SymbolGraph>>`, so writes to one project never block reads of
another.

A project moves through three states:

```
UNLOADED ──(first query)──▶ LOADING ──(snapshot load / index)──▶ ACTIVE
    ▲                          │                                    │
    │                          │  singleflight: only the first      │
    │                          │  caller loads; others wait         │
    └────────(idle unload)─────┴──── quiesce + save snapshot ◀──────┘
```

- **Singleflight loading** — concurrent requests for the same project do not
  each rebuild it. The first caller performs the load; the rest wait on a
  shared gate and use the result.
- **LRU eviction** — when the number of loaded projects exceeds
  `server.max_loaded_projects` (default 5), the least-recently-used project is
  evicted. Eviction follows a quiesce order: stop accepting new writes, let any
  in-flight heal finish, save a final snapshot, then free memory and stop
  watching.
- **Staleness check on load** — if the source tree changed since the cached
  graph was built, the entry is evicted and rebuilt before answering, so you
  never query a stale graph.

These lifecycle values live in `ProjectManagerConfig`, seeded at daemon start
from config (project-local over global); the defaults apply when the key is unset:

| Setting | Default | Config key / flag |
|---|---|---|
| `max_loaded` | 5 | `server.max_loaded_projects` |
| `max_loaded_bytes` | 0 (off) | `server.max_loaded_bytes` |
| `idle_unload` | 10 min | `server.idle_unload_minutes` |
| `auto_stop` | 30 min | `server start --auto-stop-minutes <N>` (CLI flag wins over config) |
| graceful drain | 30 s | `server.graceful_shutdown_sec` |

The idle timer resets on IPC query requests. File-watch events do not reset it
— a quiet-but-watched project still unloads on schedule.

Eviction (idle unload, LRU, or byte-budget) is persist-then-free: a graph
modified since its last snapshot is written to `.coregraph/snapshot.bin`
(atomic temp-file rename) before its memory is released, and the snapshot
records the `built_at` time. The next `LOADING` for that project warm-loads
the snapshot — skipping tree-sitter extraction — and immediately validates it:
if any source file is newer than `built_at`, the snapshot is discarded and the
graph is rebuilt from source, so a warm load never serves stale data. A clean
(unmodified) graph is dropped without a redundant write.

### Daemon self-stop

When all projects are unloaded and `auto_stop` has elapsed, the daemon drains:
it stops accepting new project loads, waits briefly for in-flight requests to
finish, then exits. The next CLI command starts it again. Override the window
with `coregraph server start --auto-stop-minutes <N>` (0 disables self-stop;
default 30).

### Running it as an OS service

If you want the daemon always resident:

```bash
coregraph server install    # register a launchd (macOS) / systemd (Linux) service
coregraph server uninstall  # remove it
```

Check status with `coregraph server status`, and start/stop/restart manually
with the matching `server` subcommands.

## In-memory graph and indexes

The graph lives entirely in process memory; nothing round-trips to a database.
That is deliberate — the core use case is feeding fresh context to an LLM (or
an IDE) with minimal latency, and an N-hop subgraph walk over an in-memory
graph is far faster than the equivalent SQL recursive query. The one weakness
of an in-memory design, cold start, is covered by snapshots.

The `SymbolGraph` (`crates/graph/src/symbol_graph.rs`) is a
`petgraph::StableGraph<SymbolNode, DirectEdge>`. `StableGraph` keeps node
indices valid across deletions, so incremental healing can remove and re-add
symbols without invalidating references held elsewhere. A
`HashMap<SymbolId, NodeIndex>` maps domain ids to petgraph indices for O(1)
lookup.

Alongside the graph, several auxiliary indexes keep queries fast:

| Index | Shape | Purpose |
|---|---|---|
| Name index | `HashMap<String, Vec<SymbolId>>` | Look up symbols by short name |
| Qualified index | `HashMap<String, Vec<SymbolId>>` | Look up by fully-qualified name |
| Value index | `HashMap<String, Vec<SymbolId>>` | Reverse-lookup string/enum values (powers cross-value inconsistency detection) |
| File blooms | per-file `SymbolBloom` | O(1) "does this file define a symbol named X?" membership test |
| Evidence index | file → evidence set | Determines the blast radius of a file change for invalidation |

Every node is a `SymbolNode` (id, kind, name, qualified_name, file, span,
status, visibility, is_test) and every edge a `DirectEdge` (from, to, kind,
origin, confidence, evidence file; its trust model is derived from the edge
kind). The edge evaluator (`edge_evaluator.rs`) computes each edge's confidence
from its kind and origin and applies stale-evidence decay — see
[confidence.md](confidence.md) for the math.

## Concurrency: RwLock plus epoch versioning

Each project's graph sits behind an `Arc<RwLock<SymbolGraph>>` (in the daemon's
project entry, and in the HTTP server's shared state). Queries take the read
lock; healing and invalidation take the write lock and mutate the graph in
place. This in-place model is the reason the graph is a `StableGraph`: node
indices stay valid as symbols are removed and re-added during a heal, so nothing
referencing them goes invalid.

To tell readers whether what they got is current, the graph carries a monotonic
version, `GraphEpoch(u64)` (`crates/graph/src/epoch.rs`). Each
invalidation-and-heal cycle returns `epoch.next()`, so the epoch increases every
time the graph changes.

### Invalidation and healing

The watcher reports changed files. Using the evidence index, the graph finds
exactly which symbols and edges those files affect and invalidates only those —
re-extracting and re-resolving them (a heal) rather than rebuilding the whole
graph — and bumps the epoch. Queries can include stale results with
`--include-stale`; by default they are healed or filtered first.

File extraction is data-parallel: the extractor (`crates/extractor`) uses rayon
to spread per-file parsing across cores.

## Snapshots and cold start

A snapshot is a `bincode` binary blob (current schema v6) at
`<project>/.coregraph/snapshot.bin`. The daemon writes one when it unloads a
project that changed since its last save, recording the build time, and
warm-loads it on the next request — skipping re-indexing unless a source file
is newer than the snapshot, in which case it rebuilds from source.

You can drive snapshots directly:

```bash
coregraph index --snapshot path/to/snapshot.bin   # write a snapshot while indexing
coregraph snapshot save --out path/to/snapshot.bin
coregraph snapshot load path/to/snapshot.bin
```

Schema v2 removed the earlier SCIP promotion layer (later versions added the
documentation layer); SCIP is no longer part of CoreGraph.

## Configuration

Two config files, both TOML:

- **Project:** `<project>/.coregraph/config.toml`, created on first index.
- **Global:** `$XDG_CONFIG_HOME/coregraph/config.toml`.

The runtime loads exactly one config file: the `--config <PATH>` if given,
otherwise the global config at `$XDG_CONFIG_HOME/coregraph/config.toml`.
`coregraph config show` displays the merged view of the global config and the
project-local `.coregraph/config.toml` (project values override global), with
the source file of each key:

```
Global config:  ~/Library/Application Support/coregraph/config.toml
Project config: ./.coregraph/config.toml

  limits.token_budget            = 8000          [project]
    # Default token budget for LLM output
  limits.hop_limit               = 3             [project]
    # Default graph traversal depth
  limits.min_confidence          = 0.7           [project]
    # Default minimum edge confidence (matches clap default)
  server.max_loaded_projects     = 5             [project]
    # Maximum projects held in the daemon cache (LRU eviction above this)
  server.graceful_shutdown_sec   = 30            [project]
    # Seconds the daemon waits for in-flight queries before hard-exit on SIGTERM
```

The HTTP listener has no config key — pass an address inline:
`coregraph server start --http 127.0.0.1:27787` (bare `--http` defaults to
`127.0.0.1:27787`; add `--allow-external` to bind a non-localhost address).

---

See also: [confidence.md](confidence.md) for the trust/confidence model and
[graph-model.md](graph-model.md) for symbol kinds, edge kinds, and analysis
origins.

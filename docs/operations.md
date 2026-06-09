# Operating CoreGraph

How to run CoreGraph day to day: the background daemon, its lifecycle commands,
auto-stop and the project cache, logging, and how the indexer degrades when a
file or a build manifest can't be parsed.

CoreGraph is meant to be invisible. You never start anything by hand for normal
use — the first `coregraph query` (or `impact`, `orphans`, etc.) spawns a
background daemon, the daemon holds the built graph in memory over a local IPC
socket, and every later command answers instantly from that cached graph. The
daemon goes away on its own once the machine goes quiet. The `server` subcommand
exists for when you want to control that lifecycle explicitly (CI, a long-lived
HTTP endpoint, or an OS-managed service).

---

## The daemon lifecycle (`coregraph server`)

```
coregraph server start      # spawn the background daemon (idempotent)
coregraph server status     # is it running? what's loaded?
coregraph server stop       # graceful shutdown
coregraph server restart    # stop + start in one command
coregraph server install    # register as an OS service (launchd / systemd)
coregraph server uninstall  # remove the OS service
```

You rarely need `start` — thin-client commands auto-spawn the daemon on first
use. `status`, `stop`, and `restart` are the ones you reach for when something
looks stuck or you want a clean slate.

### `server status`

Shows whether the daemon is running and which projects it currently holds in
memory. The format (real field layout — counts depend on your repo):

```
daemon: RUNNING
version: 0.1.0
socket: <path to IPC socket>
pid: <pid>
uptime: <N>s
Projects (1/5 loaded):
  [ACTIVE] /path/to/project — <N> symbols, <N> edges, idle <N>s, 0 in-flight
```

Per-project status is one of `ACTIVE` (graph loaded), `LOADING` (build in
progress), or `UNLOADED` (evicted from memory but still tracked). Add `--json`
for a machine-readable version of the same data.

### `server stop` and graceful shutdown

`server stop` sends `SIGTERM`. The daemon then waits up to the drain timeout
(default 30 seconds) for in-flight queries to finish before exiting, so a `stop`
issued mid-query won't truncate a response. The timeout is read at daemon start
from `server.graceful_shutdown_sec` (project-local config over global); set it to
tune how long `stop` blocks before a hard exit.

---

## Running a persistent HTTP endpoint

By default the daemon serves only the local IPC socket (used by the CLI, LSP,
and MCP bridges). To also expose the HTTP API, start with `--http`:

```
coregraph server start --http
```

With no value, `--http` binds `127.0.0.1:27787` — a deliberately uncommon port,
off the 8080/8000/3000 band, to avoid clashing with local dev servers. Pass an
explicit address to override (`--http 127.0.0.1:9120`). By default it binds
localhost only; `--allow-external` lets it bind a non-localhost interface.

```
coregraph server start --http 127.0.0.1:9120 --allow-external
```

The HTTP routes (`/health`, `/query`, `/batch`, `/api/query`, `/api/expand`,
`/api/impact`, `/api/source`) are documented in `integrations.md`. There is no
SSE or live-update stream.

---

## Auto-stop and the project cache

Three independent timers keep the daemon's footprint small, each tunable via
config (project-local over global) or, for auto-stop, a CLI flag:

| Mechanism | Default | Configurable via | What it does |
|---|---|---|---|
| Idle project unload | 10 min | `server.idle_unload_minutes` | A single project's graph is dropped from memory after it sits idle this long. Before dropping, a changed graph is persisted to `.coregraph/snapshot.bin`; the next query warm-loads it from disk (skipping re-extraction) unless a source file changed in the meantime. |
| Daemon auto-stop | 30 min | `server start --auto-stop-minutes <N>` | When *every* loaded project has been idle this long (and nothing is loading), the whole daemon exits. Dirty graphs are persisted first. |
| LRU project cap | 5 | `server.max_loaded_projects` | Maximum projects held in memory at once. Loading one over the cap evicts (and persists) the least-recently-used idle project. |
| Byte budget | unlimited | `server.max_loaded_bytes` | Approximate total heap (bytes) across all loaded graphs. Exceeding it evicts LRU idle projects until back under budget. `0` disables the byte cap. |

This is what makes the daemon feel invisible: it spins up on your first query,
serves the rest instantly, and terminates itself once you stop working.

### Auto-stop

```
coregraph server start --auto-stop-minutes 30   # default
coregraph server start --auto-stop-minutes 0    # never self-terminate
```

Auto-stop only fires on *full* idleness — any in-flight query or in-progress
load blocks it. Pass `0` to disable it entirely (useful when you've installed
the daemon as a long-lived OS service).

### LRU project eviction

When you query more than the cap (default 5) distinct projects, the daemon keeps
the most-recently-used set and evicts the rest. Eviction only targets *idle*
projects — one with an active query is never dropped mid-flight. The cap is read
at daemon start from `server.max_loaded_projects` (project-local config over
global); raise it to keep more projects resident.

---

## Installing as an OS service

`server install` registers the daemon with the platform service manager so it
survives logout and reboots:

- **macOS** — a `launchd` agent.
- **Linux** — a `systemd` unit.

```
coregraph server install     # run from inside the project you want served
coregraph server uninstall
```

Install captures the project root at registration time, so run it from the
directory you want the service to watch. On any other platform, `install`
returns an explicit error rather than silently doing nothing. Pair `install`
with `--auto-stop-minutes 0` if you want the service to stay up indefinitely.

---

## Running without a daemon

To skip the daemon entirely and build the graph in-process for a single command
— handy in CI, sandboxes, or when debugging — use `--no-auto-start` (or set
`COREGRAPH_NO_AUTO_START=1`):

```
coregraph --no-auto-start query SymbolGraph
COREGRAPH_NO_AUTO_START=1 coregraph stats
```

This trades the instant warm-cache response for a fresh build on every
invocation, but guarantees no background process is left behind.

The `--foreground` flag is the inverse: `coregraph server start --foreground`
runs the daemon *as* the current process (it does not fork). Use it under a
process supervisor or to watch the daemon's logs directly.

---

## Logging

CoreGraph logs to **stderr**, keeping stdout clean for the protocol streams that
LSP and MCP speak. Control verbosity with the global options (they apply to
every subcommand):

| Option | Effect |
|---|---|
| `--log-level <trace\|debug\|info\|warn\|error>` | Set the level explicitly. Default `info`. |
| `-v`, `--verbose` | Shortcut for `debug`. |
| `-q`, `--quiet` | Shortcut for `error` (suppresses warnings). |

Precedence is `--quiet` > `--verbose` > `--log-level`, so `-q` wins even if a
level is also given. Internally the chosen level is exported as `RUST_LOG`, so
downstream libraries honor it too. At `debug` or `trace`, CoreGraph prints a
one-line confirmation to stderr:

```
[coregraph] log level: debug
```

When you index, progress and any skipped files are reported on stderr while the
final summary goes to stdout:

```
$ coregraph index --stats
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
Index complete — 281 files, 3396 symbols, 21342 edges (2337ms)
```

---

## Graceful degradation

CoreGraph never aborts a whole index because one file or manifest is malformed.
It skips the problem, reports it, and indexes everything else. The observable
behaviors:

- **Minified / generated bundles are skipped.** Files dominated by very long
  lines (an average line over ~1000 bytes, for files larger than 4 KB) are
  treated as generated bundles — committed `*.min.js`, packed vendor files — and
  left out, because indexing them floods `orphans` and `impact` with thousands
  of meaningless single-letter symbols. The skip is announced on stderr:
  `coregraph: skipped N minified/generated file(s) (e.g. …)`. The heuristic gates
  on the *average* line length, so a real source file that merely contains one
  long base64 literal is not dropped.

- **Unreadable files are skipped.** A file that can't be read from disk is
  dropped from the batch rather than failing the index.

- **Name resolution falls back to syntactic matching.** Cross-file resolution
  runs under a per-language wall-clock budget. If a language's resolution pass
  exceeds that budget, or a reference produces no binding, CoreGraph falls back
  to tree-sitter identifier matching. Those fallback edges are honestly tagged
  `SyntaxMatched` (confidence 0.85) rather than the higher `NameResolved`, so the
  graph never claims more certainty than it has. (See `confidence.md` for the
  full origin/confidence model.)

- **A bad snapshot is rejected, not silently trusted.** `snapshot load` and
  `index --snapshot` validate the file's magic bytes and schema version. A
  corrupt or stale-schema snapshot produces an explicit error telling you to
  rebuild — `rebuild with coregraph index --snapshot` — instead of returning a
  wrong graph.

- **A changed source tree triggers a rebuild.** The daemon stamps each cached
  graph with its build time. If any source file under the project root has a
  later mtime on the next query, that cache entry is evicted and the project is
  rebuilt before answering, so you never query a stale graph.

### On-demand healing (query-path freshness)

Even between rebuilds, a query can return a point-in-time-correct view. When
healing is enabled (the default for the daemon-routed read commands — `query`,
`impact`, `inconsistencies`, and `diff`; opt out with `query --no-heal`),
CoreGraph checks the content hash of the files a query's traversal actually
touches and re-parses only those that changed, under a wall-clock budget:

- Files whose hash still matches → reused as-is.
- Files whose hash changed → re-parsed within budget.
- Files whose re-parse runs past the budget → left at their pre-heal state and
  reported with lowered trust, so you can see which paths weren't refreshed.
- Files that disappeared → reported as removed.

Healing keeps a single query honest without forcing a full reindex on every edit.

---

[Back to index](README.md)

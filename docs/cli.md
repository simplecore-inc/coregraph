# CLI Reference

`coregraph` builds an in-memory code symbol graph from your repo (tree-sitter for
symbol extraction, stack-graphs for cross-file name resolution) and lets you query
callers, callees, impact, dead code, and cross-language inconsistencies. Every edge
carries a confidence score so you (or an LLM) know how much to trust it.

This page is the full command reference. For the model behind the numbers see
[confidence.md](confidence.md) and [graph-model.md](graph-model.md).

## Quick start

```bash
# Install the CLI (puts `coregraph` on your PATH)
npm install -g @coregraph/cli

# 1. Index the repo (builds the symbol graph)
coregraph index --stats

# 2. Who calls this function?
coregraph query compute_impact --direction incoming --edge-kind calls --hop-limit 1

# 3. What breaks if I change it?
coregraph impact build_router --risk

# 4. What looks dead?
coregraph orphans --exclude-tests
```

`index --stats` prints something like:

```
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
Index complete — 281 files, 3396 symbols, 21342 edges (2337ms)
```

The first query auto-starts a background daemon and reuses the cached graph for
every later command, so subsequent queries are fast. You never have to start the
daemon by hand (see [Daemon auto-start](#daemon-auto-start)).

## Command summary

| Command | What it does |
|---|---|
| `index` | Index source files and build the symbol graph |
| `query` | Query a symbol's neighbors (callers, callees, references) |
| `inspect` | Show the symbol at `FILE:LINE` with surrounding source |
| `stats` | Graph statistics (`--breakdown` for histograms) |
| `orphans` | List orphan symbols (dead-code candidates) |
| `impact` | Impact analysis for a symbol (`--risk` adds scoring) |
| `diff` | Impact of a git diff: which symbols a change reaches |
| `review` | Auto-comment a GitHub PR with the diff impact summary |
| `inconsistencies` | Detect cross-enum / api-path / config-key / doc-drift issues |
| `export` | Export the graph as `dot`, `cypher`, or `json-graph` |
| `snapshot` | `save` / `load` a binary snapshot |
| `config` | `init` / `show` / `unset` / `path` configuration |
| `server` | Daemon mgmt: `start` `stop` `status` `restart` `install` `uninstall` |
| `lsp` | LSP stdio bridge (IDE code intelligence) |
| `mcp` | MCP stdio bridge (LLM agent tools) |
| `watch` | Watch files and rebuild the graph |
| `batch` | Run multiple queries from a JSON file |
| `plugin` | Manage plugin hooks (`list` / `run`) |

## Global options

These apply to every subcommand.

| Option | Default | Meaning |
|---|---|---|
| `-C, --project <PATH>` | `.` | Project root directory |
| `-c, --config <PATH>` | `$XDG_CONFIG_HOME/coregraph/config.toml` | Config file path |
| `--output-format <FMT>` | `human` | `human` \| `llm` \| `json` |
| `--color <WHEN>` | `auto` | `auto` \| `always` \| `never` |
| `--token-budget <N>` | `8000` | Max tokens for LLM-shaped output |
| `--hop-limit <N>` | `3` | Max graph traversal depth |
| `--min-confidence <F>` | `0.7` | Minimum edge confidence filter, `0.0`–`1.0` |
| `--include-stale` | off | Include stale nodes/edges in results |
| `--lang <LANG>` | — | Filter by language; repeatable (`java`, `rust`, …) |
| `-v, --verbose` | — | Verbose logging |
| `-q, --quiet` | — | Errors only |
| `--log-level <LEVEL>` | `info` | `trace` \| `debug` \| `info` \| `warn` \| `error` |
| `--no-auto-start` | off | Don't spawn the daemon; build in-process instead |

### `--min-confidence` in practice

The default `0.70` admits every non-pattern-matched edge: `SyntaxMatched` imports
(baseline ~0.72) and above all survive, while `PatternMatched` guesses (baseline
0.60) are filtered out.

| Value | Keeps |
|---|---|
| `0.0` | The full unfiltered graph |
| `0.70` (default) | `SyntaxMatched` and above; drops pattern guesses |
| `0.85` | Filters out `SyntaxMatched`; keeps resolved/derived edges |
| `0.90` | Keeps only `NameResolved` and `CompilerDerived` |

See [confidence.md](confidence.md) for how these numbers are computed.

### Presets

Presets are shorthand for common flag combinations — no new behavior, just less
typing.

| Preset | Expands to | Use for |
|---|---|---|
| `--fast` | `--min-confidence 0.7 --hop-limit 1 --token-budget 2000` | Quick one-hop lookups |
| `--standard` | the defaults above | Everyday use |
| `--full` | `--hop-limit 5 --include-stale --token-budget 16000` | Deep analysis, refactoring |

```bash
coregraph query UserController --fast      # 1 hop, high confidence only
coregraph impact CardService --full        # deep analysis, stale data included
```

### Output formats

| Format | For | Notes |
|---|---|---|
| `human` (default) | Terminal reading | Colors, box-drawing, an interactive pager |
| `llm` | Feeding an LLM | Token-efficient Markdown; uncertainty tagged inline |
| `json` | Scripts / pipelines | Stable schema; see [JSON shape](#json-output) |

## Commands

### `index`

```
coregraph index [OPTIONS]
```

Indexes the project and builds the graph. Incremental by default — it diffs against
the existing snapshot and only re-extracts changed files. No external toolchain
install is required.

| Flag | Meaning |
|---|---|
| `--full` | Ignore the existing snapshot and reindex everything |
| `--dry-run` | Detect changes only; don't rebuild the graph |
| `--stats` | Print file/symbol/edge counts and elapsed time |
| `--snapshot <PATH>` | Also write the resulting graph to this snapshot file |

How the graph is built:

1. **tree-sitter** extracts symbol nodes for every supported language. Syntactic
   matches are recorded as `SyntaxMatched` (confidence ~0.85).
2. **stack-graphs** resolves cross-file names into `Resolves` edges
   (`NameResolved`, confidence ~0.95). This now covers **all seven** code languages
   (see [Languages](#languages)).
3. Structurally certain edges the extractor observes directly (file→symbol
   `Contains`, symbol→module `BelongsTo`) are recorded as `CompilerDerived`
   (confidence 0.99).

### `query`

```
coregraph query <SYMBOL> [OPTIONS]
```

Looks up a symbol by exact name or substring and shows its graph neighborhood.

| Flag | Default | Meaning |
|---|---|---|
| `--kind <KIND>` | — | Filter the center symbol's kind (see [kinds](#symbol-kinds)) |
| `--direction <DIR>` | `both` | `incoming` \| `outgoing` \| `both` |
| `--edge-kind <KIND>` | — | Filter by edge kind; repeatable (see [edge kinds](#edge-kinds)) |
| `--depth <N>` | — | Traversal depth; overrides the global `--hop-limit` |
| `--aggregate` | off | Union the neighborhood across every same-name definition (recall over precision) |
| `--page-size <N>` | `50` | Page size (combined hard cap with the token budget) |
| `--cursor <TOKEN>` | — | Opaque pagination cursor from a previous response |
| `--expand <NODE_ID>` | — | Drill into one node id for detailed context |
| `--no-heal` | off | Skip on-demand healing before the query |

The clean way to list callers is to narrow direction and edge kind:

```bash
coregraph query compute_impact --direction incoming --edge-kind calls --hop-limit 1
```

```
── query: compute_impact ──────────────────────────────────

✓ compute_impact [crates/query/src/impact.rs:27]
  kind: Function | package: query (cargo)

  Incoming (14):
  ├── calls ← run [Function] @ crates/cli/src/commands/diff.rs      [0.85] ✓
  ├── calls ← run [Function] @ crates/cli/src/commands/impact.rs      [0.85] ✓
  ├── calls ← cached_impact [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← api_impact [Function] @ crates/server/src/handlers.rs      [0.85] ✓
  └── ... (14 total)
  ✓ trust: all paths verified

── page 1/1 | 14 edges total | budget: 506/5600 tokens ──
   [n]ext page | [e]xpand <id> | [f]ilter --edge-kind | [q]uit
```

The last two lines are the **interactive pager** (human format only). When results
span more than one page you can press `n` for the next page, `e <id>` to expand a
node, `f` to filter by edge kind, or `q` to quit. The budget readout (`506/5600`)
shows tokens used against the effective budget — the effective budget is the
advertised `--token-budget` scaled by a 0.7 safety margin, since token counts are
estimated from byte counts rather than a real tokenizer.

> `query` is the command that takes `--depth`. The depth flag on `impact` is named
> `--max-depth` — they are not interchangeable.

### `inspect`

```
coregraph inspect <FILE:LINE> [OPTIONS]
```

Shows the symbol(s) covering a source location, with surrounding code.

| Flag | Default | Meaning |
|---|---|---|
| `--context-lines <N>` | `5` | Surrounding source lines to include |

```bash
coregraph inspect crates/query/src/impact.rs:27
```

```
── inspect: crates/query/src/impact.rs:27 ──
  compute_impact [Function] bytes 926..2903
  doc::compute_impact [DocComment] bytes 507..926

      22 /// The conceptual "impact of X" spans both directions: X's callers (incoming —
      23 /// who breaks if X changes) and X's callees (outgoing — what X depends on).
  →   27 pub fn compute_impact(graph: &SymbolGraph, seed_id: SymbolId, max_depth: usize) -> ImpactResult {
      28     let mut visited: HashSet<SymbolId> = HashSet::new();
```

### `stats`

```
coregraph stats [OPTIONS]
```

| Flag | Default | Meaning |
|---|---|---|
| `--breakdown` | off | Symbol/edge kind histograms, per-crate counts, top in-degree symbols, heaviest files |
| `--top <N>` | `20` | Top-N cut-off for breakdown lists |

```bash
coregraph stats
```

```
symbols: 3396
edges: 21357
```

`stats --breakdown --top 8` adds histograms:

```
Indexed 281 files
symbols: 3396
edges:   21342

## Symbol kinds
  Function         1191
  DocComment       593
  Method           459
  ...

## Edge kinds
  Resolves         7669
  Calls            4365
  Contains         2262
  ...

## Analysis origins
  SyntaxMatched        9237
  NameResolved         6699
  CompilerDerived      4524
  PatternMatched       861
  ConventionInferred   21
```

### `orphans`

```
coregraph orphans [OPTIONS]
```

Lists symbols with no incoming or outgoing edges — dead-code candidates.

| Flag | Default | Meaning |
|---|---|---|
| `--public-only[=true\|false]` | `true` | Report only public symbols; pass `--public-only=false` to also include private ones (higher-confidence dead code) |
| `--exclude-tests` | off | Exclude symbols from test files/directories |

```bash
coregraph orphans --exclude-tests
```

```
Orphan symbols (12): 7 likely dead, 5 library API surface, 0 test code
  as_kebab [Method] — crates/cli/src/commands/query.rs
  strip_api_path_prefix [Function] — crates/extractor/src/string_literal_extractor.rs [library API]
  unregister [Method] — crates/graph/src/hooks.rs [library API]
  outputChannel [Constant] — vscode-extension/src/extension.ts
```

A `[library API]` tag marks a public symbol — it has no internal callers but may be
called from outside the crate, so it is lower-confidence "dead."

### `impact`

```
coregraph impact <SYMBOL> [OPTIONS]
```

Computes which symbols are reachable from (affected by) a change to `<SYMBOL>`.

| Flag | Default | Meaning |
|---|---|---|
| `--max-depth <N>` | `5` | Maximum impact propagation depth |
| `--transitive` | off | Compute the transitive closure (requires `--max-depth`) |
| `--risk` | off | Add confidence-weighted risk scoring |

> The depth flag here is `--max-depth`, not `--depth`.

`--risk` adds a blast-radius score, a confidence-weighted impact total, and the set
of affected tests:

```bash
coregraph impact build_router --risk
```

```
Impact of 'build_router': 1251 reachable symbols, 1251 edges, depth 3
  Risk Score: 0.96 (Critical)
  Blast Radius: Critical (16 modules, 910 callers)
  Confidence-Weighted Impact: 653.500
  Affected tests: 334
    test_app (distance 2, path_confidence 0.90) — ./crates/server/src/handlers.rs
    create_app_returns_router (distance 2, path_confidence 0.90) — ./crates/server/src/lib.rs
    ... (more affected tests)
  post [Method] — ./crates/graph/src/hooks.rs
  create_app [Function] — ./crates/server/src/lib.rs
  ... (reachable symbols listed)
```

The risk score blends visibility, direct-caller count, module spread, and impact
kind into a 0–1 value classified Low / Medium / High / Critical. See
[graph-model.md](graph-model.md) for the exact weights and thresholds.

### `diff`

```
coregraph diff <BASE> [OPTIONS]
```

Maps a git diff onto the graph: which symbols the change touches, and which symbols
those reach.

| Flag | Default | Meaning |
|---|---|---|
| `--to <REF>` | `HEAD` | Compare this ref instead of the working tree |
| `--max-depth <N>` | global `--hop-limit` | Impact propagation depth from each touched symbol |
| `--exclude-tests` | off | Skip symbols defined under test directories |

```bash
coregraph diff HEAD~1 --exclude-tests
```

```
coregraph: skipped 1 minified/generated file(s) (e.g. vscode-extension/media/cytoscape.min.js)
Diff HEAD~1..HEAD: 52 file(s), 974 touched symbol(s), 1659 reachable (depth 3)
  • reindex_latency.rs [File] @ crates/cli/examples/reindex_latency.rs
  • main [Function] @ crates/cli/examples/reindex_latency.rs
  … and 954 more
```

### `review`

```
coregraph review --pr <N> [OPTIONS]
```

Posts (or prints) a GitHub PR comment summarizing the diff impact. The PR number is
inferred from the repo via `gh pr view`; the `gh` CLI must be authenticated.

| Flag | Default | Meaning |
|---|---|---|
| `--pr <N>` | required | PR number in the current repo |
| `--dry-run` | off | Print the comment body to stdout instead of posting |
| `--max-depth <N>` | `3` | Impact propagation depth from each touched symbol |
| `--exclude-tests` | off | Skip symbols defined under test directories |

```bash
coregraph review --pr 42 --dry-run
```

### `inconsistencies`

```
coregraph inconsistencies [OPTIONS]
```

Detects cross-language drift the compiler can't see.

| Flag | Meaning |
|---|---|
| `--category <CAT>` | Restrict to one category |

Categories:

| Category | Detects |
|---|---|
| `enum-mismatch` | The same value appearing under different enums/roles |
| `api-path` | The same API path declared in places that disagree |
| `config-key` | Config keys that don't line up across files |
| `doc-drift` | A `@param`/`:param` naming a parameter the signature no longer has (JS/TS/Java/Python) |

```bash
coregraph inconsistencies
```

```
Inconsistencies (63):
  [enum-mismatch] 'admin' appears in:
    - Permission.ADMIN (./tests/e2e/golden/04-inconsistencies/src/permissions.ts)
    - Role.ADMIN (./tests/e2e/golden/04-inconsistencies/src/roles.ts)
  [api-path] /a.rs vs /b.rs
    ...
```

On a repo that contains test fixtures (like this one) the raw output includes
fixture noise. Narrow with `--category` to focus.

### `export`

```
coregraph export [OPTIONS]
```

| Flag | Default | Meaning |
|---|---|---|
| `--format <FMT>` | `dot` | `dot` \| `cypher` \| `json-graph` |
| `--subgraph <SYMBOL>` | — | Restrict to a subgraph centered on this symbol (+`--hop-limit` hops) |

```bash
coregraph export --format dot --subgraph build_router > graph.dot
```

### `snapshot`

```
coregraph snapshot save --out <PATH>
coregraph snapshot load <FILE>
```

`save` indexes the project and writes a binary snapshot; `load` reads one back and
prints its summary. Snapshots are bincode blobs (schema v6).

| Subcommand | Flag/arg | Meaning |
|---|---|---|
| `save` | `-o, --out <PATH>` (required) | Output snapshot path |
| `load` | `<FILE>` (positional) | Snapshot file to load |

### `config`

```
coregraph config <init|show|unset|path>
```

| Subcommand | Meaning |
|---|---|
| `init` | Create a default config file at the configured path |
| `show` | Print the effective config (on-disk values + defaults) |
| `unset <KEY>` | Remove a key |
| `path` | Print the config file path |

A legacy positional form (`coregraph config <KEY> <VALUE>`) still works for writing
a single key.

```bash
coregraph config show
```

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

Per-project config lives at `<project>/.coregraph/config.toml` (created on first
index); global config at `$XDG_CONFIG_HOME/coregraph/config.toml`. Config files use
`[limits]`, `[server]`, and `[index]` sections — `[index] exclude = [...]` accepts
gitignore-style patterns.

### `server`

```
coregraph server <start|stop|status|restart|install|uninstall> [OPTIONS]
```

Manage the background daemon directly. You normally don't need this — queries
auto-start the daemon — but it's here for explicit control and HTTP exposure.

| Subcommand | Meaning |
|---|---|
| `start` | Start the daemon (detached by default) |
| `stop` | Stop the running daemon (SIGTERM + drain) |
| `status` | Show daemon status (add `--json` for machine output) |
| `restart` | Stop + start in one command |
| `install` | Register the daemon as an OS service (launchd on macOS, systemd on Linux) |
| `uninstall` | Remove the OS service registration |

`server start` options:

| Flag | Default | Meaning |
|---|---|---|
| `--http [<ADDR>]` | off | Also expose an HTTP API; bare `--http` binds `127.0.0.1:27787` |
| `--allow-external` | off | Allow binding to non-localhost interfaces |
| `--foreground` | off | Run in the foreground (the process is the daemon itself) |
| `--auto-stop-minutes <N>` | `30` | Self-terminate after N idle minutes; `0` disables |

```bash
coregraph server start --http               # bind 127.0.0.1:27787
coregraph server start --http 127.0.0.1:9120
coregraph server status --json
```

### `watch`

```
coregraph watch [OPTIONS]
```

Watches the project and rebuilds the graph on change.

| Flag | Meaning |
|---|---|
| `--diff` | Show the graph diff (before/after) on each rebuild |
| `--no-incremental` | Force a full rebuild on each change instead of incremental invalidate+heal |

### `batch`

```
coregraph batch <QUERIES_FILE> [OPTIONS]
```

Runs many symbol queries from a single JSON file (an array of names) in one daemon
round-trip.

```json
["compute_impact", "build_router", "query_symbol"]
```

```bash
coregraph batch queries.json --output-format json
```

### `plugin`

```
coregraph plugin <list|run>
```

| Subcommand | Meaning |
|---|---|
| `list` | List all registered plugin hooks |
| `run` | Dry-run the default registry against a directory, firing all pre/post hooks |

### `lsp` and `mcp`

```
coregraph lsp     # LSP stdio bridge — your IDE launches this
coregraph mcp     # MCP stdio bridge — your LLM client launches this
```

Both are lightweight stdio bridges: they connect to the daemon over IPC (starting
it if needed) and translate protocol. The daemon holds the graph; the bridge just
relays. See [Integrations](#integrations).

## Daemon auto-start

Thin-client commands (`query`, `impact`, `orphans`, `inconsistencies`, `stats`,
`inspect`, …) connect to a background daemon over a Unix domain socket. On the first
command:

```
coregraph query build_router
  │
  ├─ IPC socket present? ── yes ─→ connect → send query → return result
  │
  └─ no → auto-start enabled?
          ├─ yes → spawn the daemon (detached), wait for the socket, then query
          └─ no  → fall back to an in-process build_graph for this one command
```

The daemon caches the graph so later queries skip the rebuild. It evicts projects
beyond `server.max_loaded_projects` (default 5, LRU) and self-terminates after
`--auto-stop-minutes` of full idleness (default 30).

To suppress auto-start:

- `--no-auto-start` — one command, build in-process instead
- `COREGRAPH_NO_AUTO_START=1` — for the whole session

## Symbol kinds

`--kind` accepts: `function`, `method`, `class`, `struct`, `interface`, `trait`,
`enum`, `enum-variant`, `constant`, `variable`, `field`, `type-alias`, `module`,
`namespace`, `config-key`, `string-literal`, `package`, `external-package`.

## Edge kinds

`--edge-kind` accepts: `resolves`, `calls`, `implements`, `extends`, `overrides`,
`references`, `imports`, `string-match`, `configures`, `depends-on`.

## Languages

**Symbol extraction (tree-sitter):** Rust, Java, Kotlin, TypeScript, JavaScript, Go,
Python — plus config files (YAML / TOML / JSON → `ConfigKey` nodes) and Markdown
(the documentation layer).

**Cross-file name resolution (stack-graphs):** all seven code languages.

- Upstream stack-graphs rules: Java, TypeScript, JavaScript, Python
- Hand-authored `.tsg` rules (`crates/stack/rules/{go,rust,kotlin}.tsg`): Go, Rust,
  Kotlin

Resolution falls back to tree-sitter syntactic matching only when a language has no
rules at all, or when resolution produces no binding.

## JSON output

`--output-format json` produces a stable shape. A trimmed `query` example:

```json
{
  "query": "compute_impact",
  "center": {
    "id": 1296, "name": "compute_impact", "kind": "Function",
    "file": "crates/query/src/impact.rs", "span_start": 926, "span_end": 2903,
    "context": { "package": "query (cargo)", "generated": false, "generator": null }
  },
  "edges": [
    {
      "direction": "incoming", "kind": "calls", "depth": 1,
      "other_id": 40, "other_name": "run",
      "confidence": 0.8549999594688416,
      "trust": "NameResolved", "origin": "NameResolved",
      "trust_model": "SourceEvidenced",
      "stale_evidence_count": 0, "current_confidence": 0.95
    }
  ]
}
```

Each edge carries `confidence` (computed at index time), `origin`/`trust`
(how the edge was derived), `trust_model`, `stale_evidence_count`, and
`current_confidence` (after decay). See [confidence.md](confidence.md).

## Integrations

### MCP (LLM agents)

`coregraph mcp` is a JSON-RPC stdio server exposing `initialize`, `tools/list`, and
`tools/call`. It exposes exactly five tools (plain names, no prefix):

| Tool | Input | Returns |
|---|---|---|
| `query` | `{ "name": string }` | Symbols matching the name |
| `impact` | `{ "name": string, "depth": integer = 5 }` | Transitive impact for a symbol name |
| `orphans` | `{}` | Symbols with no incoming or outgoing edges |
| `inconsistencies` | `{}` | Cross-enum value mismatches |
| `stats` | `{}` | Graph summary: nodes, edges, file count |

Register it with a Claude Code `.mcp.json` (or `claude_desktop_config.json`):

```json
{ "mcpServers": { "coregraph": { "command": "coregraph", "args": ["mcp"] } } }
```

### LSP (IDE)

`coregraph lsp` is a stdio LSP bridge. It advertises:

| Capability | Request |
|---|---|
| `definitionProvider` | `textDocument/definition` |
| `referencesProvider` | `textDocument/references` |
| `workspaceSymbolProvider` | `workspace/symbol` |

### HTTP API

Start it with `coregraph server start --http [ADDR]`. The default bind is
`127.0.0.1:27787` (off the common 8080/8000/3000 band). Use `--allow-external` to
bind a non-localhost interface.

| Method | Route | Params / body | Returns |
|---|---|---|---|
| `GET` | `/health` | — | `{ status, version, symbol_count }` |
| `POST` | `/query` | `{ name, limit=50 }` | `{ name, count, symbols[] }` |
| `POST` | `/batch` | `{ queries: [name, …] }` | `{ results: [...] }` |
| `GET` | `/api/query` | `?symbol=&page=0&page_size=50&budget=8000` | `{ query, matches[], pagination, budget }` |
| `GET` | `/api/expand` | `?node=<id>&budget=2000` | `{ node, incoming[], outgoing[], budget }` |
| `GET` | `/api/impact` | `?symbol=&depth=5` | `{ symbol, depth, reachable_count, edge_count, nodes[] }` |
| `GET` | `/api/source` | `?file=&line=1&context=5` | `{ file, target_line, context_lines, total_lines, snippet[] }` |

```bash
curl http://127.0.0.1:27787/health
curl 'http://127.0.0.1:27787/api/query?symbol=SymbolGraph&page=0&page_size=50'
```

---

[Back to docs index](README.md)

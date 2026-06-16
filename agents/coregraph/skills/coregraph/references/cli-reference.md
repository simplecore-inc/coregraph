# coregraph CLI reference

Global options (available on every subcommand):

| Option | Default | Meaning |
|---|---|---|
| `-C, --project <PATH>` | `.` | Target project root |
| `-c, --config <FILE>` | `$XDG_CONFIG_HOME/coregraph/config.toml` | Config file path |
| `--output-format <F>` | `human` | `human` / `llm` / `json` |
| `--color <M>` | `auto` | `auto` / `always` / `never` |
| `--token-budget <N>` | `8000` | LLM output token cap |
| `--hop-limit <N>` | `3` | Graph traversal depth |
| `--min-confidence <F>` | `0.7` | Edge confidence filter. `0.7` drops `PatternMatched`; `0.85` also drops `SyntaxMatched`; `0.9` keeps only `NameResolved` and `CompilerDerived` — but real `NameResolved` `calls` edges sit at ~0.85, so any threshold above 0.85 drops most callers (don't raise above `0.85`); `0.0` is the full graph |
| `--include-stale` | off | Include stale nodes/edges in results |
| `--lang <L>` | — | Language filter (repeatable): `java`, `rust`, `typescript`, `python`, `go`, … |
| `-v` / `-q` / `--log-level` | — | Logging |
| `--fast` / `--standard` / `--full` | — | Presets. `--fast` = hop 1 / budget 2000; `--full` = hop 5 / budget 16000 + include-stale |
| `--no-auto-start` | off | Don't auto-spawn the daemon when the IPC socket is missing; fall back to in-process `build_graph`. Env: `COREGRAPH_NO_AUTO_START=1` |

CoreGraph builds the graph from two layers in one `index` pass: **tree-sitter** (symbol
extraction) and **stack-graphs** (cross-file name resolution). No external compiler index
or language server is required.

---

## index — build the symbol graph (and save a snapshot)

```bash
coregraph index [OPTIONS]
```

- `--full` — ignore the existing snapshot and reindex everything.
- `--dry-run` — detect changes only; don't rebuild the graph.
- `--stats` — print file / symbol / edge counts and elapsed time.
- `--snapshot <PATH>` — save the resulting graph to a binary snapshot. The path is resolved
  relative to your **shell's cwd, not to `-C`**, and the daemon only warm-loads
  `<project>/.coregraph/snapshot.bin` — so anchor the path there (an unanchored relative path
  run from another cwd silently disables warm-load).

```bash
# First index, or refresh the snapshot (snapshot path anchored to the project)
coregraph -C /path/to/repo index --stats --snapshot /path/to/repo/.coregraph/snapshot.bin

# Full rebuild ignoring any prior snapshot
coregraph index --full --stats
```

---

## query — look up symbols by name

```bash
coregraph query [OPTIONS] <SYMBOL>
```

- `<SYMBOL>` — exact name or substring. Multiple matches return a candidate list.
- `--kind <K>` — restrict the center symbol: `function` / `method` / `class` / `struct` /
  `interface` / `trait` / `enum` / `enum-variant` / `constant` / `variable` / `field` /
  `type-alias` / `module` / `namespace` / `config-key` / `string-literal` / `package` /
  `external-package`.
- `--direction incoming|outgoing|both` (default `both`).
- `--edge-kind <E>` — repeatable: `resolves` / `calls` / `implements` / `extends` /
  `overrides` / `references` / `imports` / `string-match` / `configures` / `depends-on`.
- `--aggregate` — union the neighborhood across every same-name definition instead of
  centering on one. Use for genuine overloads, or when you want callers of *every* symbol
  sharing the name (recall over precision).
- `--depth <N>` — override the hop limit for this query.
- `--page-size <N>` (default 50), `--cursor <OPAQUE>` — pagination.
- `--expand <NODE_ID>` — drill down into a specific node.
- `--no-heal` — skip on-demand re-extraction of stale evidence files (200 ms budget) before
  querying.

The default hop-limit is `3`, so an incoming `calls` query returns **transitive** callers
too. For a literal "who directly calls X" answer, pin `--depth 1` (or `--fast`); omit it for
the full transitive reach.

```bash
coregraph query DisruptorPipeline --kind class --direction incoming --edge-kind calls --depth 1
coregraph query handleRequest --aggregate --direction incoming
```

---

## inspect — look up the symbol at a location

```bash
coregraph inspect [OPTIONS] <FILE:LINE>
```

- `--context-lines <N>` (default 5) — show surrounding source lines.

```bash
coregraph inspect src/main.rs:42
coregraph inspect apps/gateway/src/GatewayApp.java:120 --context-lines 10
```

---

## stats — graph statistics

```bash
coregraph stats [OPTIONS]
```

- `--breakdown` — symbol/edge kind histograms, analysis-origin histogram, trust-model
  histogram, per-crate counts, top in-degree symbols, and heaviest files.
- `--top <N>` (default 20) — breakdown cut-off.

```bash
coregraph stats --breakdown --top 15
```

---

## orphans — isolated symbols (dead-code candidates)

```bash
coregraph orphans [OPTIONS]
```

- `--exclude-tests` — exclude test files/directories.
- `--public-only [=true|false]` — **defaults to `true`** (report only public symbols).
  Pass `--public-only=false` to also include private symbols (higher-confidence dead code).

The result is already restricted to **real code symbols** (functions, methods, classes,
structs, interfaces, traits, enums, constants, variables, fields, type-aliases, namespaces);
config keys, string literals, and doc/container nodes are excluded internally and never
appear. So no `ConfigKey`/`StringLiteral` post-filtering is needed — the bracket labels in
the output are symbol *kinds* (`[Method]`, `[Function]`, …), not `[ConfigKey]`. The output is
pre-classified (`Orphan symbols (N): X likely dead, Y library API surface, Z test code`);
read the likely-dead rows and confirm each with a targeted read.

**Recall ceiling:** `orphans` reports only **fully-disconnected** symbols (no semantic edge in
*either* direction). A dead symbol that still has any resolved *outgoing* edge — a never-called
function that itself calls a live helper, a dead component that renders other components — is
**not** reported. An empty result is therefore not proof of "no dead code"; treat the list as
triage candidates. To suppress generated/noise files from this report *without* turning the
symbols they reference into false orphans, list them under `[analysis] exclude` (kept indexed,
own symbols hidden) rather than `[index] exclude` (dropped entirely, edges lost).

---

## impact — change impact for a single symbol

```bash
coregraph impact [OPTIONS] <SYMBOL>
```

- `--transitive` — compute the transitive closure (requires `--max-depth`).
- `--max-depth <N>` (default 5) — propagation depth.
- `--risk` — emit Risk Score (4-factor), Blast Radius, Confidence-Weighted Impact, and
  Affected tests.

Example output:

```
Impact of 'DisruptorPipeline': 159 reachable symbols, 222 edges, depth 3
  Risk Score: 0.90 (Critical)
  Blast Radius: Critical (45 modules, 2439 callers)
  Confidence-Weighted Impact: 1955.665
  Affected tests: 1205
```

---

## diff — impact of a git diff

```bash
coregraph diff [OPTIONS] <BASE>
```

- `<BASE>` — git ref: `main`, `HEAD~3`, a commit SHA, or a tag.
- `--to <REF>` — compare this ref instead of the working tree. Default `HEAD` (so
  working-tree edits are included).
- `--max-depth <N>` — propagation depth from each touched symbol.
- `--exclude-tests` — skip symbols under test directories.

```bash
coregraph diff main --exclude-tests
coregraph diff HEAD~10 --to HEAD --max-depth 2
```

---

## inconsistencies — cross-file inconsistency detection

```bash
coregraph inconsistencies [OPTIONS]
```

- `--category <C>` — `enum-mismatch` / `api-path` / `config-key` / `doc-drift`.
  - `enum-mismatch` — the same variant value/name defined in two different **code** enums
    (an accidental collision, or a divergent copy across files/languages — e.g. `Permission.ADMIN`
    and `Role.ADMIN` both `"admin"`). Not an external-data (DB/proto/YAML) comparison.
  - `api-path` — pairwise (O(n²)) match over path-like **string literals**; short
    slash-prefixed strings (including mock paths in test fixtures) produce false hits. Not
    reliably "a real API mismatch" — verify provenance.
  - `config-key` — config keys with no resolved code binding; accuracy is project-dependent
    (false positives from camelCase↔snake/kebab normalization or reflection binding). Not
    categorically noisier than `api-path`.
  - `doc-drift` — a `@param` / `:param` naming a parameter the signature no longer has
    (JS/TS/Java/Python).

There is **no `--exclude-tests` flag** here. Judge hits by **provenance first**: in
`--output-format json`, check each hit's `a.file`/`b.file` (the matched value is
`a.name`/`b.name`) and discard pairs where both sides are under `tests/`, `fixtures/`,
`__fixtures__/`, or `*.test.*`. Count distinct *production*
files, not the raw hit total. The four categories are project-dependent — rank them by
inspecting actual hits, not by a fixed order. To suppress fixture noise at the source, add
those paths to `[index] exclude` in `.coregraph/config.toml` and re-index.

```bash
coregraph inconsistencies --category enum-mismatch
coregraph inconsistencies --category api-path --output-format json
coregraph inconsistencies --category doc-drift
```

---

## export — emit the graph in an external format

```bash
coregraph export [OPTIONS]
```

- `--format dot|cypher|json-graph` (default `dot`).
- `--subgraph <SYMBOL>` — restrict to a hop-limit radius around the symbol. Without it you
  get the whole graph, which is rarely practical.

```bash
coregraph export --format dot --subgraph GatewayApp > gateway.dot && dot -Tsvg gateway.dot -o gateway.svg
coregraph export --format cypher > graph.cypher
coregraph export --format json-graph > graph.json
```

---

## snapshot — save / load a snapshot manually

```bash
coregraph snapshot save <PATH>
coregraph snapshot load <PATH>
```

Same format as `index --snapshot`. `load` prints a summary.

---

## config — configuration management

```bash
coregraph config init                  # create a default config file
coregraph config show                  # print effective (on-disk + defaults) config
coregraph config path                  # print the config file path
coregraph config unset <KEY>           # remove a key
coregraph config recommend [--write]   # graph-derived config.toml tuning (see below)
coregraph config <KEY> [VALUE]         # legacy positional read / write
```

`config recommend` analyzes the indexed graph and prints suggested `.coregraph/config.toml`
tuning — data-dominated files for `[index] exclude` (i18n/locale bundles,
`resources/messages/*.properties`, generated JSON schemas), a `[index] string_match_max_files`
cap, optional `api-path` category disabling, and generated-file `[analysis] exclude`
candidates. Suggestion-only; `--write` merges them into the file (comment-preserving).

Common keys:

| Key | Purpose |
|---|---|
| `limits.token_budget` | Default LLM output token budget |
| `limits.hop_limit` | Default traversal depth |
| `limits.min_confidence` | Default edge confidence cut |
| `server.max_loaded_projects` | Daemon LRU cache slots |
| `server.graceful_shutdown_sec` | Drain time on SIGTERM |
| `index.exclude` | Array of gitignore-style patterns for files **not parsed at all** (no nodes, no edges — cuts symbols/memory, but a symbol referenced only by an excluded file becomes a false orphan). **Not surfaced by `config show`** — read it via `coregraph config index.exclude` (legacy positional) or open the file |
| `analysis.exclude` | Array of gitignore-style patterns for files **kept indexed** (their edges keep referents connected) but whose own symbols are **hidden from dead-code (`orphans`) reports**. Prefer over `index.exclude` for generated consumers like `routeTree.gen.ts`. Open the file to edit |

A per-project config is auto-created at `<project>/.coregraph/config.toml`. Note `config show`
prints only the `limits.*` / `server.*` keys above; `index.exclude` lives in the file but is
not echoed by `config show`.

---

## server — daemon management

```bash
coregraph server start   [--http [addr]] [--allow-external] [--foreground] [--auto-stop-minutes <N>]
coregraph server stop      # SIGTERM + drain
coregraph server status
coregraph server restart
coregraph server install   # register as launchd (macOS) / systemd (Linux) service
coregraph server uninstall
```

- `--http` with no value binds `127.0.0.1:27787`.
- `--allow-external` is required to bind a non-localhost interface (secure default).
- `--auto-stop-minutes 0` disables idle self-shutdown.

The daemon auto-starts on the first thin-client command (`query` / `impact` / …) unless
`--no-auto-start` is set.

---

## viz — 3D graph viewer (atlas) over local HTTP

```bash
coregraph viz [--port 7321] [--no-open] [--detach] [--stop] [--html <FILE>]
```

Serves the bundled 3D symbol-graph viewer on `127.0.0.1` (default port `7321`), auto-spawning
the daemon and streaming the graph from memory. `--detach` runs it in the background (output to
`viz.log`, recorded in `viz.pid`) and returns once the port answers; `--stop` terminates the
detached instance. Loopback-only and CSRF/Host-checked (localhost tooling).

---

## watch — watch for changes and rebuild incrementally

```bash
coregraph watch [OPTIONS]
```

- `--diff` — show the before/after snapshot diff on each change.
- `--no-incremental` — full rebuild on every change (default is invalidate + heal).

---

## batch — run multiple queries from a JSON file

```bash
coregraph batch <QUERIES_FILE>
```

`QUERIES_FILE` is a JSON array of symbol names. `batch` resolves each name and returns a JSON
count/match list per entry (`{name, count, symbols:[matched names]}`) — a bulk
**existence / disambiguation** check, **not** a batched `query`: it returns no edges,
callers, or neighborhood, and always emits JSON (it ignores `--output-format`). For callers
or impact of many symbols, loop `query` / `impact` per symbol.

---

## review — auto-comment a GitHub PR

```bash
coregraph review --pr <N> [--dry-run] [--max-depth <N>] [--exclude-tests]
```

Posts a diff-impact summary as a comment on the PR. `--dry-run` prints to stdout instead.
`--pr` can be inferred via `gh pr view`.

---

## plugin — plugin hooks

```bash
coregraph plugin list
coregraph plugin run <DIR>     # dry-run the default registry
```

---

## lsp / mcp — stdio bridges

```bash
coregraph lsp      # LSP stdio — editor integration
coregraph mcp      # MCP stdio — LLM agent tools
```

Both speak standard protocols. For how an LLM agent should drive coregraph (and the MCP
fast-path), see [`llm-usage.md`](llm-usage.md).

# Data Flow (End-to-End)

This page traces how a file moves from disk through indexing into a query
response. Read it when you want to know *why* an edge has the confidence it
does, or *when* the graph reflects an edit you just made.

There are three flows:

1. **Initial indexing** — the cold build that turns a project tree into a graph.
2. **Runtime change handling** — how an edit on disk reaches the cached graph.
3. **Query response** — how `coregraph query` (or an MCP/HTTP call) turns a
   symbol name into a trust-tagged answer.

You can watch the first flow happen:

```
$ coregraph index --stats
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
Index complete — 281 files, 3396 symbols, 21342 edges (2337ms)
```

---

## 1. Initial indexing

`coregraph index` runs one pass over the project tree. Each stage adds nodes or
edges to a single in-memory `SymbolGraph`; later stages read what earlier ones
produced. The order matters — for example, structural File/Module nodes are
created before reference resolution so that a top-of-file import has an
enclosing symbol to attach to.

```
project directory
    │
    ▼
manifest detection + parse        package.json, Cargo.toml, build.gradle, go.mod, …
  → module boundaries, internal vs. external dependencies, exclude/include set
    │
    ▼
generated-code detection          build-plugin declarations + file markers
  → keep real source, skip minified/generated files
    │
    ▼
file scan                         collect source files per language
  → code files + config files (YAML/TOML/JSON) + Markdown docs
    │
    ▼
tree-sitter parse + extract       every file → AST → SymbolNode (parallel, per file)
  → Function, Method, Struct, Class, Interface, Enum, Constant, ConfigKey, …
    │
    ▼
merge                             per-file node sets stitched into one graph
    │
    ▼
value matching                    cross-file string/value links (StringMatch)
    │
    ▼
mediator detection                DI / config patterns → Configures edges
  → Spring DI, Spring config, React Router, Docker Compose, Go DI
    │
    ▼
structural pass                   File/Module nodes + Contains / BelongsTo edges
    │
    ▼
documentation layer               DocComment + DocSection nodes
  → Documents (doc→symbol), Mentions (intra-doc link), DescribedIn (md→symbol)
    │
    ▼
typed references                  Calls / Imports / Extends / Implements
    │
    ▼
stack-graphs name resolution      cross-file binding for all 7 languages
  → promotes stitched hits to NameResolved (0.95)
    │
    ▼
edge reclassification + types     StringMatch → EnumValueMatch / ApiPathMatch;
                                  TypeOf / GenericParam from source text
    │
    ▼
snapshot (bincode, schema v6)     optional, with --snapshot <PATH>
```

### What each stage contributes

| Stage | Adds | Notes |
|---|---|---|
| Manifest detection | project structure | Determines module boundaries and the exclude/include set. The `[index] exclude` config key (gitignore syntax) feeds this. |
| File scan | the file set | Code, config (YAML/TOML/JSON → `ConfigKey` nodes), and Markdown. |
| tree-sitter extract | `SymbolNode`s | Runs every language extractor in parallel, one scratch graph per file. Extractors add **nodes only** — edges come from later stages. |
| Merge | one graph | Per-file node sets are copied into the main graph. |
| Value matching | `StringMatch` edges | Links identical string literals across files (e.g. an API path string and its route handler). A value found in more than `[index] string_match_max_files` distinct files (default 8, 0 = unlimited) is skipped — convention strings would otherwise emit O(k²) hub edges. |
| Mediators | `Configures` edges | Framework-specific resolvers. These produce `ExternallyMediated` edges. See [graph-model.md](graph-model.md). |
| Structural pass | `Contains` / `BelongsTo` | Creates File and Module nodes so every symbol has an enclosing scope. |
| Documentation layer | `Documents` / `Mentions` / `DescribedIn` | Doc comments and Markdown sections become nodes linked to the code they describe. |
| Typed references | `Calls` / `Imports` / `Extends` / `Implements` | Edges the extractors emit from syntax. |
| Stack-graphs resolution | promotes to `NameResolved` | Cross-file name binding. Hits it can stitch are upgraded to confidence 0.95; the rest stay at the syntactic level (0.85). |

### Language coverage at the resolution stage

Cross-file name resolution runs **stack-graphs for all seven code languages**:

- **Upstream stack-graphs rules**: Java, TypeScript, JavaScript, Python.
- **CoreGraph hand-authored `.tsg` rules** (`crates/stack/rules/{go,rust,kotlin}.tsg`):
  Go, Rust, Kotlin.

A name that stack-graphs binds becomes a `NameResolved` edge (0.95). When
resolution does not produce a binding, the edge stays at the tree-sitter
syntactic level (`SyntaxMatched`, 0.85). Config files and Markdown have no
stack-graphs rules; their edges come from the value-matching and documentation
stages instead.

> Origins and confidence are explained in [confidence.md](confidence.md). The
> short version: `CompilerDerived` (0.99) > `NameResolved` (0.95) >
> `SyntaxMatched` (0.85) > `PatternMatched` (0.60) > `ConventionInferred` (0.40).

After the cold build, the graph is held in the daemon's memory (or written to a
snapshot with `--snapshot`). You don't re-run `index` for every query — the
daemon serves them.

---

## 2. Runtime change handling

When the daemon is watching a project (`coregraph watch`, or any thin-client
command that auto-starts the daemon), edits on disk update the cached graph
incrementally instead of triggering a full reindex.

```
file watcher event
    │
    ▼
debounce                          coalesce a burst of events (100ms window),
                                  drop paths excluded by config (target/, node_modules/, …)
    │
    ▼
content-hash check                confirm the file actually changed (ignore touch-only events)
    │
    ▼
incremental rebuild               re-extract changed files, then re-run the
                                  downstream edge stages (value match, mediators,
                                  references, stack-graphs resolution)
    │
    ▼
epoch bump                        graph version counter increments
```

Two details worth knowing:

- **Why downstream stages re-run.** A changed *definition* can invalidate edges
  in files that didn't change (an import target moved, a string literal that two
  files shared changed). CoreGraph keeps the graph correct by re-extracting only
  the changed files (the parallel, file-local stage) and then re-running the
  cross-file edge stages. The saving comes from skipping extraction of unchanged
  files, not from skipping resolution.
- **The epoch.** Every invalidate-and-rebuild cycle bumps a monotonic
  `GraphEpoch` counter. Queries can tell whether the graph they read is the same
  version as a previous answer.

If you don't want a background watcher, run with `--no-auto-start` (or
`COREGRAPH_NO_AUTO_START=1`) and CoreGraph builds the graph in-process for that
one command.

---

## 3. Query response

A query — from the CLI, the MCP bridge, or the HTTP API — resolves against the
cached graph without rebuilding it.

```
query: "what depends on compute_impact, and what does it reach?"
    │
    ▼
name lookup                       resolve the symbol name → node id(s)
                                  (exact match first, then fuzzy)
    │
    ▼
subgraph extraction               N-hop BFS over edges from the center node,
                                  bounded by --hop-limit / --depth
    │
    ▼
on-demand healing                 for files on the BFS path whose hash changed,
                                  re-extract within a wall-clock budget; files
                                  that miss the budget are flagged stale
    │
    ▼
confidence filter                 drop edges below --min-confidence (default 0.70);
                                  decay confidence by stale-evidence count
    │
    ▼
pagination + token budget         page the edge list (--page-size, default 50)
                                  and cap output at --token-budget (default 8000)
    │
    ▼
trust tagging + serialize         tag each edge with its trust model, render as
                                  human / llm / json (--output-format)
```

### On-demand healing

The watch loop may lag behind a fast edit. Healing closes that gap on the read
path so a query returns a point-in-time-correct view even when the watcher
hasn't caught up:

1. Collect the evidence files on the BFS path from the seed symbol.
2. Compare each file's on-disk content hash against the remembered state.
3. For files whose hash changed, re-extract — under a total wall-clock budget.
4. Files whose healing would exceed the budget are flagged: the answer still
   reflects their pre-heal state, and edges evidenced by them are reported with
   reduced trust rather than silently trusted.

Disable healing per query with `--no-heal` if you want the cached graph exactly
as-is.

### Pagination and the token budget

Large fan-outs are paged, not truncated silently. The CLI shows the page footer:

```
── page 1/1 | 14 edges total | budget: 506/5600 tokens ──
   [n]ext page | [e]xpand <id> | [f]ilter --edge-kind | [q]uit
```

- `--page-size <N>` (default 50) sets edges per page; `--cursor <…>` resumes from
  a page.
- `--token-budget <N>` (default 8000) caps serialized output so an LLM context
  isn't blown by one query. The `--fast` preset lowers it to 2000; `--full`
  raises it to 16000.
- `--expand <id>` pulls the neighbors of one node from the previous result
  without re-running the whole query.

### Trust tagging

Every edge in the answer carries its origin, confidence, trust model, and a
`current_confidence` that reflects stale-evidence decay. In `json` output:

```json
{
  "direction": "incoming", "kind": "calls", "depth": 1,
  "other_id": 40, "other_name": "run",
  "confidence": 0.85,
  "trust": "NameResolved", "origin": "NameResolved",
  "trust_model": "SourceEvidenced",
  "stale_evidence_count": 0, "current_confidence": 0.95
}
```

In `human` output the same information collapses to a trailing score and a check
mark — `[0.85] ✓`. See [confidence.md](confidence.md) for what the numbers mean
and [graph-model.md](graph-model.md) for the trust models.

---

[Back to index](README.md)

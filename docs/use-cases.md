# CoreGraph Use Cases

Task-first scenarios for using CoreGraph in a real development workflow. Every
command and output below is real — captured by running `coregraph` on its own
repository (a Rust workspace under `crates/`). Your own project's numbers will
differ, but the shapes are identical.

If you haven't indexed yet, do that first:

```
coregraph index --stats
```
```
coregraph: skipped 1 minified/generated file(s) (e.g. ./vscode-extension/media/cytoscape.min.js)
Index complete — 281 files, 3396 symbols, 21342 edges (2337ms)
```

---

## 1. Give an LLM agent accurate code context (MCP)

### Situation
An AI coding agent (Claude Code, Cursor, …) is about to change code and needs to
know the related symbols and dependencies. grep-based search misses renames,
re-exports, and cross-language references.

### How
Wire CoreGraph into the agent over MCP. Add this to `.mcp.json` (Claude Code) or
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "coregraph": {
      "command": "coregraph",
      "args": ["mcp"]
    }
  }
}
```

`coregraph mcp` speaks JSON-RPC over stdio (`initialize`, `tools/list`,
`tools/call`) and reuses the running daemon when one is up. It exposes exactly
five tools:

| Tool | Input | What it answers |
|---|---|---|
| `query` | `{ "name": string }` | Look up symbols by name across the project |
| `impact` | `{ "name": string, "transitive": boolean = false, "depth": integer = 5 }` | Dependents of a symbol — direct (depth-1) by default, full transitive closure when `transitive` is true |
| `orphans` | `{}` | Symbols with no incoming or outgoing edges |
| `inconsistencies` | `{}` | Cross-enum value mismatches |
| `stats` | `{}` | Graph summary: nodes, edges, file count |

Tool names are plain (no `coregraph_` prefix). The agent calls them on its own
when it needs the graph.

### Why it helps
- The agent learns who calls a function before it edits the signature.
- Cross-language references are followed (e.g. a React `fetch` URL → a Spring
  controller route), not just same-language symbols.
- Every edge carries a confidence score and trust model, so the agent can tell a
  compiler-verified call from a heuristic string match and act accordingly.

---

## 2. Impact analysis before a change

### Situation
You're about to change an interface signature or delete an endpoint and need to
know how far the blast reaches.

### How

```
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
    batch_endpoint_returns_results_array (distance 3, path_confidence 0.86) — ./crates/server/src/handlers.rs
    ... (more affected tests)
  post [Method] — ./crates/graph/src/hooks.rs
  create_app [Function] — ./crates/server/src/lib.rs
  ... (reachable symbols listed)
```

`--risk` adds the risk score, blast radius, confidence-weighted impact, and the
list of affected tests. To walk the full transitive closure, combine
`--transitive` with `--max-depth`:

```
coregraph impact build_router --transitive --max-depth 5
```

Note the flag is `--max-depth` (the `impact` command has no `--depth`). The risk
score classifies the change: `< 0.4` Low, `0.4–0.6` Medium, `0.6–0.8` High,
`> 0.8` Critical.

### Why it helps
- The confidence-weighted impact discounts low-trust edges, so the number
  reflects realistic reach rather than every speculative match.
- Affected tests are identified automatically — you know exactly what to run.
- The risk score gives a single number to gate a PR on.

---

## 3. Find dead code

### Situation
A legacy module has accumulated functions and classes nobody calls. You need to
know which are safe to delete.

### How

```
coregraph orphans --exclude-tests
```
```
Orphan symbols (12): 7 likely dead, 5 library API surface, 0 test code
  as_kebab [Method] — crates/cli/src/commands/query.rs
  strip_api_path_prefix [Function] — crates/extractor/src/string_literal_extractor.rs [library API]
  strip_config_ref_prefix [Function] — crates/extractor/src/string_literal_extractor.rs [library API]
  unregister [Method] — crates/graph/src/hooks.rs [library API]
  outputChannel [Constant] — vscode-extension/src/extension.ts
  IpcEventDisposable [Interface] — vscode-extension/src/ipc/client.ts
  POSITION_CACHE_TTL_MS [Constant] — vscode-extension/src/providers/statusBarProvider.ts
  size [Method] — vscode-extension/src/util/lruMap.ts
```

The summary line splits orphans into **likely dead**, **library API surface**,
and **test code**. The `[library API]` tag marks public symbols that may be a
deliberate API surface (lower-confidence "dead") rather than truly unused.

`--public-only` defaults to `true`, so only public symbols are reported. To also
include private symbols, pass `--public-only=false`:

```
coregraph orphans --public-only=false --exclude-tests
```

### Why it helps
- This is reference analysis over the resolved graph, not a text search — renames
  and re-exports don't produce false "dead" results.
- The `[library API]` tag and the dead/library/test split tell you which orphans
  warrant a closer look before deletion.
- Cross-language references are counted, so a symbol used only from another
  language isn't flagged as dead.

---

## 4. Detect cross-language inconsistencies

### Situation
In a monorepo, a backend enum value and a frontend string constant drift apart,
or an API path differs between server and client.

### How

```
coregraph inconsistencies
```
```
Inconsistencies (63):
  [enum-mismatch] 'admin' appears in:
    - Permission.ADMIN (./tests/e2e/golden/04-inconsistencies/src/permissions.ts)
    - Role.ADMIN (./tests/e2e/golden/04-inconsistencies/src/roles.ts)
  [api-path] /a.rs vs /b.rs
    - ./vscode-extension/test/unit/diagnosticsProvider.test.ts
    - ./vscode-extension/test/unit/diffImpactGraph.test.ts
  ...
```

Narrow to one class of issue with `--category`:

```
coregraph inconsistencies --category enum-mismatch
```

The four categories are:

| `--category` | Detects |
|---|---|
| `enum-mismatch` | The same value declared in two different enums/constants |
| `api-path` | Server route vs client request path that don't match |
| `config-key` | A config key referenced one way in YAML/TOML and another in code |
| `doc-drift` | A `@param` / `:param` naming a parameter the signature no longer has |

(CoreGraph's own repo includes test-fixture noise, as above — a real project's
output is cleaner. Use `--category` to focus, or persist the choice with
`[inconsistencies] disable = ["api-path"]` in `.coregraph/config.toml`;
an explicit `--category` still runs a disabled category.)

### Why it helps
- This is semantic matching on the values themselves, not a string search you
  could do with grep.
- Cross-language value tracking validates the contract between server and client.
- Run it in CI to block drift before it reaches runtime.

---

## 5. Review a pull request

### Situation
On a PR you want the impact of just the changed lines, and the side effects that
are easy to miss.

### How
Get the impact of a git diff:

```
coregraph diff HEAD~1 --exclude-tests
```
```
coregraph: skipped 1 minified/generated file(s) (e.g. vscode-extension/media/cytoscape.min.js)
Diff HEAD~1..HEAD: 52 file(s), 974 touched symbol(s), 1659 reachable (depth 3)
  • reindex_latency.rs [File] @ crates/cli/examples/reindex_latency.rs
  • main [Function] @ crates/cli/examples/reindex_latency.rs
  • find_workspace_root [Function] @ crates/cli/examples/reindex_latency.rs
  ...
  … and 954 more
```

`diff <BASE>` compares `BASE..HEAD` by default (override the end with `--to`),
reports the touched symbols and everything reachable from them, and accepts
`--max-depth` and `--exclude-tests`.

To post the summary as a comment on a GitHub PR, use `review`:

```
coregraph review --pr 42
```

`review --pr <N>` runs the same diff-impact analysis for that PR and comments on
it. Use `--dry-run` to print the comment without posting, and `--max-depth` /
`--exclude-tests` to scope it.

### Why it helps
- The reviewer sees the real reach of the change instead of just the diff.
- `--exclude-tests` keeps the focus on production impact.
- `review --pr` puts the summary where the discussion already is.

---

## 6. Find every caller of a symbol

### Situation
Before touching a function you want a clean list of who calls it — only direct
callers, not the full neighborhood.

### How

```
coregraph query compute_impact --direction incoming --edge-kind calls --hop-limit 1
```
```
── query: compute_impact ──────────────────────────────────

✓ compute_impact [crates/query/src/impact.rs:27]
  kind: Function | package: query (cargo)

  Incoming (14):
  ├── calls ← run [Function] @ crates/cli/src/commands/diff.rs      [0.85] ✓
  ├── calls ← run [Function] @ crates/cli/src/commands/impact.rs      [0.85] ✓
  ├── calls ← run [Function] @ crates/cli/src/commands/review.rs      [0.85] ✓
  ├── calls ← cached_impact [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← dispatch_diff_with_git [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← cached_impact_batch [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← dispatch_impact [Function] @ crates/cli/src/dispatch.rs      [0.85] ✓
  ├── calls ← api_impact [Function] @ crates/server/src/handlers.rs      [0.85] ✓
  └── ... (14 total)
  ✓ trust: all paths verified

── page 1/1 | 14 edges total | budget: 506/5600 tokens ──
   [n]ext page | [e]xpand <id> | [f]ilter --edge-kind | [q]uit
```

The three flags do the narrowing:
- `--direction incoming` → only callers (use `outgoing` for callees, `both` is the default).
- `--edge-kind calls` → only call edges (drop imports, references, etc.). Repeatable.
- `--hop-limit 1` → direct callers only, no transitive walk.

The `[0.85]` is the edge confidence and `✓` marks a verified path. Each result
also shows the file and line, so you can jump straight to it.

### Why it helps
- A clean, direct-caller list instead of a tangled multi-hop neighborhood.
- Confidence per edge tells you which callers are compiler-verified.
- For machine consumption, add `--output-format json` (see the JSON shape below).

---

## 7. Onboard onto a new codebase

### Situation
A new team member needs to grasp the shape of a large codebase fast: which kinds
of symbols dominate, how dense the graph is, and what a given file does.

### How
Start with a breakdown of the whole graph:

```
coregraph stats --breakdown --top 8
```
```
Indexed 281 files
symbols: 3396
edges:   21342

## Symbol kinds
  Function         1191
  DocComment       593
  Method           459
  File             238
  ExternalPackage  202
  ConfigKey        150
  Struct           148
  StringLiteral    92
  DocSection       59
  Interface        48
  Enum             46
  Class            44
  Module           42
  Constant         26
  TypeAlias        22
  EnumVariant      19
  Field            11
  Trait            6

## Edge kinds
  Resolves         7669
  Calls            4365
  Contains         2262
  BelongsTo        2262
  Imports          1745
  References       1297
  Documents        593
  TypeOf           574
  ApiPathMatch     208
  DescribedIn      149
  GenericParam     133
  Implements       48
  Configures       21
  EnumValueMatch   10
  Mentions         5
  Extends          1

## Analysis origins
  SyntaxMatched        9237
  NameResolved         6699
  CompilerDerived      4524
  PatternMatched       861
  ConventionInferred   21

## Trust models
  SourceEvidenced  20198
  ContractDependent 756
  Bidirectional    367
  ExternallyMediated 21
```

`--breakdown` adds the histograms (symbol kinds, edge kinds, analysis origins,
trust models — shown in full); `--top <N>` caps the trailing per-symbol /
per-file ranking lists. The analysis-origin and trust-model histograms tell you
at a glance how much of the graph is compiler-verified versus heuristically
inferred.

Then inspect a specific location to see the symbol there plus surrounding source:

```
coregraph inspect crates/query/src/impact.rs:33
```
```
── inspect: crates/query/src/impact.rs:33 ──
  compute_impact [Function] bytes 1128..3581
  doc::compute_impact [DocComment] bytes 531..1128

      31 /// this repo's graph via shared callees), not an impact measure. What X itself
      32 /// depends on (outgoing) does not break when X changes.
      ...
  →   33 pub fn compute_impact(graph: &SymbolGraph, seed_id: SymbolId, max_depth: usize) -> ImpactResult {
      34     let mut visited: HashSet<SymbolId> = HashSet::new();
      ...
```

`inspect FILE:LINE` resolves the symbol at that line and shows source context
(`--context-lines <N>`, default 5). It also surfaces the attached `DocComment`.

### Why it helps
- You get the structural shape of the codebase without reading a line of source.
- The origin/trust histograms show how trustworthy the graph is for this project.
- `inspect` ties a line number directly to its symbol and docs.

---

## 8. Trace a config key into code

### Situation
You're about to change `application.yml`, `docker-compose.yml`, or another config
file and need to know which code reads each key.

### How
Config keys are first-class `ConfigKey` nodes. Query one by name with the
`config-key` kind:

```
coregraph query "spring.datasource.url" --kind config-key
```

The neighbors show where the key is bound in code. Config-to-code bindings are
produced by CoreGraph's cross-language mediators (Spring config, Spring DI,
React Router, Docker Compose, Go DI) and carry the `ExternallyMediated` trust
model — meaning the link goes through an external configuration file rather than
direct source evidence, so it's tracked at lower confidence and re-checked when
the mediating file changes.

To catch keys that are spelled one way in config and another in code, use:

```
coregraph inconsistencies --category config-key
```

### Why it helps
- You see the impact of a config change before you make it.
- The `ExternallyMediated` trust model makes the config-to-code link explicit and
  its confidence visible.
- `--category config-key` flags drift between a key's declaration and its use.

---

## 9. Cross-language code intelligence in your IDE (LSP)

### Situation
A single-language IDE language server only understands one language at a time.
CoreGraph's LSP bridge adds graph-backed navigation across the whole monorepo.

### How
Register `coregraph lsp` as a language server in your editor. It advertises:

| Capability | LSP request |
|---|---|
| Go to Definition | `textDocument/definition` |
| Find References | `textDocument/references` |
| Workspace Symbol | `workspace/symbol` |

### Why it helps
- Definition and reference lookups follow the resolved graph, including
  cross-language edges a single-language server would miss.
- Workspace-symbol search spans every indexed language at once.

---

## 10. Wire CoreGraph into CI

### Situation
Before a merge you want to fail on cross-language inconsistencies and watch for
growing dead code, automatically.

### How
Run the analyses with JSON output and gate on the result:

```yaml
- name: CoreGraph analysis
  run: |
    coregraph index --stats
    coregraph inconsistencies --output-format json > inconsistencies.json
    coregraph orphans --output-format json > orphans.json
```

For an external system to query the graph over HTTP, start the daemon with an
HTTP listener:

```
coregraph server start --http
```

With no address, `--http` binds `127.0.0.1:27787`. The API includes:

| Method | Route | Returns |
|---|---|---|
| GET | `/health` | `{status, version, symbol_count}` |
| POST | `/query` | `{name, count, symbols[]}` |
| GET | `/api/impact` | `{symbol, depth, reachable_count, edge_count, nodes[]}` |
| GET | `/api/source` | `{file, target_line, context_lines, total_lines, snippet[]}` |

Use `--allow-external` to bind a non-localhost address.

### Why it helps
- Cross-language inconsistencies are blocked before they reach runtime.
- The HTTP API lets dashboards and other tooling read the same graph the CLI uses.

---

## Reference: machine-readable output

Add `--output-format json` to any query for a stable shape. A `query` result
looks like:

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

To visualize a dependency subgraph, export it for Graphviz:

```
coregraph export --subgraph build_router --format dot > subgraph.dot
```

`export` also supports `--format cypher` and `--format json-graph`.

---

## Scenario summary

| Scenario | Command / tool | Core value |
|---|---|---|
| LLM context | MCP (`query`, `impact`, `orphans`, `inconsistencies`, `stats`) | Precise cross-language context |
| Impact analysis | `impact <sym> --risk --transitive --max-depth` | Confidence-weighted risk scoring |
| Dead code | `orphans --exclude-tests` | Resolved-graph dead-code detection |
| Inconsistencies | `inconsistencies --category …` | Cross-language value matching |
| PR review | `diff <base>`, `review --pr <N>` | Impact of just the changed lines |
| Find callers | `query <sym> --direction incoming --edge-kind calls --hop-limit 1` | Clean direct-caller list |
| Onboarding | `stats --breakdown`, `inspect FILE:LINE` | Structural understanding fast |
| Config tracing | `query "<key>" --kind config-key` | ExternallyMediated binding tracking |
| IDE intelligence | LSP (`coregraph lsp`) | Cross-language go-to-definition |
| CI integration | JSON output + HTTP API | Automated quality gates |

---

[Back to index](README.md)

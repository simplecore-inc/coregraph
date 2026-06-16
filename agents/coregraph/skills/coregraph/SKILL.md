---
name: coregraph
description: >-
  Use the `coregraph` CLI/MCP — a code symbol graph (tree-sitter + stack-graphs) — as the
  PRIMARY tool for structural and relational code questions, in preference to a raw
  grep/read sweep. Trigger when the user asks to find callers / who uses a symbol / where a
  symbol is defined / what it calls or depends on / how it is wired up, analyze change
  impact or blast radius, find dead code / orphans / unused symbols, detect cross-file
  inconsistencies (enum / api-path / config-key / doc drift), see the impact of a git diff,
  get a structural symbol-graph overview (symbol counts, top files, edge breakdown via
  `stats`), or explicitly invokes coregraph — e.g. "who calls X", "where is X defined",
  "what does X depend on", "what breaks if I change X", "impact 분석", "dead code 찾아줘",
  "누가 이거 호출해", "orphan 찾아줘", "cross-file 불일치 검사", "coregraph로 분석",
  "symbol graph 뽑아줘", "코드 그래프로 보여줘". Do NOT trigger for reading the logic inside a
  single function, for non-symbol content (comments, string contents, config values, prose,
  TODO hunting), or for a general narrative "what does this project do / explain this repo"
  overview — read the README or source for those.
---

# coregraph

`coregraph` indexes a codebase into one queryable **symbol graph** — tree-sitter extracts
symbols, stack-graphs resolves names across files — and answers structural questions
(callers, impact, dead code, cross-file consistency) from the precomputed graph instead of
re-reading files. A caller lookup that would otherwise mean pasting several files lands in a
few hundred tokens.

**This skill is the single source of usage guidance for coregraph.** The kit's `AGENTS.md`
and the per-agent integration files are thin wrappers that point here; the deep material
lives in the bundled references listed at the end of this skill.

## When to prefer coregraph over grep/read (the core decision)

Treat coregraph as the **primary** tool for *structural / relational* questions, and your
default file tools (grep / read / glob) for *logic / content* questions.

**Prefer coregraph** when the question is:

| Question | Why it wins over grep/read | Command |
|---|---|---|
| Who calls / uses X? | Resolves the real binding across files; text search overcounts name collisions and misses aliased imports | `coregraph query X --direction incoming --edge-kind calls --depth 1` |
| Where is X defined / what does it call? | Jumps to the real definition and the outgoing neighborhood | `coregraph query X --direction outgoing` · `coregraph inspect FILE:LINE` |
| What breaks if I change X? | Transitive closure + risk + the tests it touches | `coregraph impact X --risk` |
| What does my git change affect? | Maps touched lines → symbols → blast radius | `coregraph diff <base> --exclude-tests` |
| What is dead / unused? | Graph in/out-degree; separates likely-dead from public API | `coregraph orphans --exclude-tests` |
| Where do these disagree? (enum / api-path / doc) | Cross-file consistency you'd otherwise read everything to find | `coregraph inconsistencies --category <cat>` |
| Repeated structural nav in a large / polyglot repo | Daemon answers from memory; reading files blows the budget | any of the above |

**Use grep/read instead** for: reading the actual logic inside a function/file, tiny repos,
non-symbol content (comments, string contents, config values, prose, TODO/FIXME), a general
"what does this project do" narrative overview, or constructs coregraph can't resolve
(reflection, dynamic dispatch, macros, unsupported languages).

**Bias + caveat:** once indexed, reach for coregraph first on any structural/relational
question; fall back to reading files only for concrete logic or non-symbol text. **Always
verify a surprising or negative result** ("no callers" / "dead code") with a targeted read —
dynamic references and missing edges cause false positives.

**Reflective frameworks (Spring/JPA/CDI/Nest) — what to trust.** Method-to-method `calls`
edges resolve well (a service method shows its real intra-code callers at ~0.85), so
`query`/`impact` at **method** granularity and `stats --breakdown` are trustworthy. But
**class/type-level DI wiring is invisible** to the graph: `impact <ServiceClass>` under-reports
(measured **0 callers / Risk 0.20 Low** for a `@Service` that a controller injects). Never
conclude a `@Service`/`@Repository`/`@Controller` is unused from `impact`/`orphans` alone —
rely on the structural map, method-level navigation, and the cleaner inconsistency categories.

## Setup (once)

```bash
# Binary on PATH (if `which` fails after install, the npm global bin dir isn't on PATH —
# run by absolute path, e.g. "$(npm prefix -g)/bin/coregraph", or add that dir to PATH):
which coregraph || npm install -g @coregraph/cli

# Index once. --snapshot is resolved relative to your SHELL'S cwd, not to -C, and the daemon
# only warm-loads <project>/.coregraph/snapshot.bin — so anchor the path there:
coregraph -C <project> index --stats --snapshot <project>/.coregraph/snapshot.bin
```

Indexing **always honors `.gitignore`** (even without a `.git` directory) and **auto-excludes
build outputs and dependency dirs** by default — `build/`, `dist/`, `out/`, `target/`,
`node_modules/`, `.gradle/`, `vendor/`, `__pycache__/`, `.venv/`, `venv/`, plus `.git/`,
`.idea/`, `.vscode/`. You do NOT need to list these in `[index] exclude`. For non-source
*data* a `.gitignore` misses (committed i18n/locale bundles, `resources/messages/*.properties`,
generated JSON schemas, vendored sample trees), add them to `[index] exclude` — they otherwise
inflate the graph with thousands of `ConfigKey`/`StringLiteral` nodes (and the noise lands in
`orphans`/`inconsistencies`). `coregraph config recommend` proposes these excludes from the
graph; `--write` merges them.

A background daemon auto-starts on the first query and serves later queries from memory; the
snapshot warm-loads next session. Index once, then just query. If the plugin's MCP server is
connected, the `query` / `impact` / `orphans` / `inconsistencies` / `stats` / `recommend`
tools are available natively — `diff` / `inspect` / `export` / `review` / `viz` and the
filtering flags are **CLI-only**, so shell out for those.

## Command cheat-sheet

All commands accept `--output-format human|llm|json` (default `human`); use `llm` to hand
results to a model. The `--fast` / `--standard` / `--full` presets bundle hop limit, token
budget **and `--min-confidence`** — the confidence change is the consequential part:
- `--fast` = `--min-confidence 0.9 --hop-limit 1 --token-budget 2000`. ⚠ 0.9 keeps only
  `NameResolved`/`CompilerDerived` and drops most `calls` edges (which sit at ~0.85), so
  `--fast` can return an **empty "no callers"** result. Use it for cheap overviews, not callers.
- `--standard` = defaults (min-confidence 0.7, hop 3, budget 8000).
- `--full` = `--min-confidence 0.0 --hop-limit 5 --include-stale --token-budget 16000`. 0.0
  exposes even `PatternMatched` (heuristic) edges — maximal recall, more noise.

| Goal | Command |
|---|---|
| Symbol lookup (partial match OK) | `coregraph query <Name>` (`--kind class`, `--direction incoming`, `--edge-kind calls`) |
| Direct callers only | `coregraph query <Name> --direction incoming --edge-kind calls --depth 1` (omit `--depth` for transitive callers, hop-limit 3) |
| Location lookup | `coregraph inspect path/to/file.rs:42` |
| Structural overview | `coregraph stats --breakdown --top 15` |
| Change impact | `coregraph impact <Name> --risk` |
| Impact of a git diff | `coregraph diff HEAD~5 --exclude-tests` |
| Dead code | `coregraph orphans --exclude-tests` |
| Cross-file inconsistencies | `coregraph inconsistencies --category enum-mismatch` |
| Visualize a subgraph | `coregraph export --format dot --subgraph <Name>` |
| Auto-comment a PR | `coregraph review --pr <N> --exclude-tests` |
| Recommend config tuning | `coregraph config recommend [--write]` (graph-derived `[index]`/`[analysis]` excludes, `string_match_max_files` cap, api-path toggle) |
| 3D graph viewer (atlas) | `coregraph viz [--port 7321] [--detach] [--stop]` (local HTTP on 127.0.0.1) |
| Save / load a snapshot | `coregraph snapshot save\|load` |

The full per-command flag set is [`references/cli-reference.md`](references/cli-reference.md).

## Reading results — trust tiers

Every edge carries a confidence score and an origin. Trust tiers, highest to lowest:
**`CompilerDerived`** (compiler-grade) › **`NameResolved`** (stack-graphs, scope-accurate) ›
**`SyntaxMatched`** (tree-sitter, syntactic) › **`PatternMatched`** (heuristic) ›
**`ConventionInferred`** (config-convention, low-volume). `--min-confidence` (default `0.7`)
drops `PatternMatched`; `0.85` also drops `SyntaxMatched`.

**Don't raise `--min-confidence` above 0.85 to "tighten" callers.** Real `NameResolved`
`calls` edges sit at **~0.85** (measured), so any threshold above 0.85 (e.g. `0.9`) drops
them and you get an empty "no callers" answer. Keep the default `0.7` (or at most `0.85`);
use `0.0` for the full graph. `impact --risk` blends in-degree, transitive reach, test coverage, and confidence
into a 0–1 Risk Score (`≥0.85` Critical) plus a Blast Radius.

## Interpreting results — signal vs. noise

- **`orphans` already returns only real code symbols** (functions, methods, classes, structs,
  interfaces, traits, enums, constants, variables, fields, type-aliases, namespaces). Config
  keys, string literals, and doc/container nodes are excluded internally, so they never
  appear — **no `ConfigKey`/`StringLiteral` pre-filtering is needed or possible.** The output
  is pre-classified; the header (e.g. `Orphan symbols (10): 7 likely dead, 3 library API
  surface, 0 test code`) tells you which rows to read. Narrow with `--exclude-tests` and
  `--public-only` (default `true`; pass `--public-only=false` to add private symbols as
  higher-confidence dead code). Always confirm a hit with a targeted read — dynamic dispatch,
  reflection, FFI, serialization, and macro/derive-generated usage (e.g. clap `#[derive(Args)]`
  / `ValueEnum`) are out-of-graph and cause false "dead" hits.
  - **⚠ Reflective frameworks (Spring / JPA / CDI / NestJS-style) — the orphans list is
    FP-DOMINATED, not a dead-code census.** Symbols whose only inbound wiring is reflective are
    orphans *by construction*: `@RestController`/`@Controller` (HTTP entrypoints — the framework
    dispatches them, no in-code caller exists), `@Service`/`@Component`/`@Repository`
    (constructor-DI; Spring Data `@Repository` interfaces have no source impl at all),
    `@Bean` factories, `@Scheduled`/`@EventListener` handlers, `@ConfigurationProperties`
    classes, and DTO/entity classes + their fields (used only across the Jackson/JPA
    serialization boundary). A controller/service usually surfaces as its **constructor
    (`kind=Method`)** — seeing `*RestController [Method]` / `*Service [Method]` rows is the
    *signature* of a DI false positive, not a dead constructor. On a measured 921-file Spring
    monorepo, **16 of 16 spot-checked orphans were framework FPs (0 dead)**. Treat the bulk list
    as near-zero-signal there; never report these as removable without a targeted source read.
  - **Recall ceiling — `orphans` finds only FULLY-DISCONNECTED symbols.** A symbol is reported
    only when it has no semantic edge in *either* direction; a dead symbol that still has any
    resolved *outgoing* edge (a never-called function that itself calls a live helper, a dead
    component that renders other components) is **not** reported. So a clean/empty result is not
    proof there is no dead code — the list is triage candidates, not a census.
  - **Index-exclude vs analysis-exclude (config).** `[index] exclude` drops files from indexing
    entirely (no nodes *and no edges*) — good for cutting symbol count, but a symbol referenced
    **only** by an excluded file then shows up as a *false* orphan (classic case: excluding
    `routeTree.gen.ts` orphans every `export const Route`). To suppress generated/noise files
    from dead-code reports *without* orphaning what they reference, list them under
    `[analysis] exclude` instead — those files stay indexed (their edges keep referents
    connected) but their own symbols are hidden from `orphans`.
- **`inconsistencies` — judge by provenance first, then category.** There is **no
  `--exclude-tests` flag** here. A **default run covers only `enum-mismatch` + `api-path` +
  `config-key`**; `doc-drift` is **opt-in** (`--category doc-drift`) and never appears
  otherwise. The categories are project-dependent, not a fixed trust ranking:
  - **JSON shape.** Envelope is `{count, reports:[…]}`. `enum-mismatch`/`api-path`/`config-key`
    hits are pairwise: `{category, shared_value, a:{name,file,line}, b:{name,file,line}}` — use
    `a.file`/`b.file` for the provenance check below. `doc-drift` is a **single-node** shape:
    `{category, symbol, file, detail, candidates}` (no `a`/`b`) — use its `file`.
  - **Provenance (pairwise categories only).** Discard any pair where both sides live under
    `tests/`, `fixtures/`, `__fixtures__/`, or `*.test.*` — these often dominate self-analysis.
    Count distinct *production* files, not the raw hit total.
  - `config-key` has two sub-kinds in `shared_value`: **`missing-key:`** (code references a key
    absent from config — e.g. `@Value`/`process.env`/`os.environ`) and **`unused-key:`** (a
    config-file key with no matching code reference). ⚠ **On Spring (or any
    `@Value`/`@ConfigurationProperties`/env-bound system) `config-key` is overwhelmingly false
    positives and is by far the noisiest category** — measured **304 hits / >90% FP** on a
    921-file monorepo (vs 2 `api-path`). The cause is structural, not just casing: a key read
    via `@Value("${key:default}")` or bound through a `@ConfigurationProperties(prefix=…)` class
    is resolved by the framework from external YAML/env at runtime, so coregraph sees a code
    reference with no in-graph binding — **even when the key exists in `application.yml`**.
    Framework-native keys (`spring.*`, `logging.level.*`, `management.*`, `springdoc.*`) are
    flagged the same way. Do **not** treat the bulk `config-key` list as a misconfiguration report.
  - `api-path` matches path-like **string literals** pairwise (O(n²)). On a REST app it yields
    **sibling-route pairs** that share a URL shape but are different resources (e.g.
    `/admin/user/note` vs `/admin/user/role`) — **not** mismatches. A real hit is singular/plural
    or version drift of the **same** route between a client and a server. Short slash-prefixed
    strings (incl. test mock paths) also produce false hits.
  - `enum-mismatch` / `doc-drift` are the **reliable** categories (on the Spring repo
    `enum-mismatch` was correctly empty and `doc-drift` surfaced a real Javadoc/overload
    mismatch). Still apply the provenance check.
  - To suppress data/fixture noise at the source, add those paths to `[index] exclude` (or
    `coregraph config recommend`) and re-index.

## Where to look (references)

| Need | Reference |
|---|---|
| Every subcommand and flag | [`references/cli-reference.md`](references/cli-reference.md) |
| Step-by-step analysis of an unfamiliar repo, interpretation criteria | [`references/analysis-workflow.md`](references/analysis-workflow.md) |
| How an LLM should drive coregraph (the `--output-format llm` path, MCP fast-path) | [`references/llm-usage.md`](references/llm-usage.md) |
| Daemon races, PATH issues, empty queries, reset | [`references/troubleshooting.md`](references/troubleshooting.md) |

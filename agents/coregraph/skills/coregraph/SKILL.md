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

## Setup (once)

```bash
# Binary on PATH (if `which` fails after install, the npm global bin dir isn't on PATH —
# run by absolute path, e.g. "$(npm prefix -g)/bin/coregraph", or add that dir to PATH):
which coregraph || npm install -g @coregraph/cli

# Index once. --snapshot is resolved relative to your SHELL'S cwd, not to -C, and the daemon
# only warm-loads <project>/.coregraph/snapshot.bin — so anchor the path there:
coregraph -C <project> index --stats --snapshot <project>/.coregraph/snapshot.bin
```

A background daemon auto-starts on the first query and serves later queries from memory; the
snapshot warm-loads next session. Index once, then just query. If the plugin's MCP server is
connected, the `query` / `impact` / `orphans` / `inconsistencies` / `stats` tools are
available natively — `diff` / `inspect` / `export` / `review` and the filtering flags are
**CLI-only**, so shell out for those.

## Command cheat-sheet

All commands accept `--output-format human|llm|json` (default `human`); use `llm` to hand
results to a model. The `--fast` / `--standard` / `--full` presets set hop limit + token
budget together (`--fast` = hop 1 / 2000, `--standard` = defaults, `--full` = hop 5 / 16000
+ stale).

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
  `--exclude-tests` flag** here, and the four categories are project-dependent, not a fixed
  trust ranking:
  - For each hit, look at its two source files (`a.file`/`b.file` in `--output-format json`;
    the matched value is `a.name`/`b.name`).
    **Discard any pair where both sides live under `tests/`, `fixtures/`, `__fixtures__/`, or
    `*.test.*`** — these often dominate self-analysis. Count distinct *production* files, not
    the raw hit total.
  - `api-path` matches path-like **string literals** pairwise (O(n²)), so short
    slash-prefixed strings — including mock paths in tests like `{ file: "/a.rs" }` — produce
    false hits. A real one is a route literal shared between a production client and a
    production server (singular/plural or version drift). It is **not** reliably "a real API
    mismatch."
  - `config-key` reports config keys with no resolved code binding; accuracy varies by repo
    (false positives mainly from camelCase↔snake/kebab normalization or reflection-based
    binding). It is **not** categorically noisier than `api-path`.
  - `enum-mismatch` / `doc-drift` are usually cleanest, but still apply the provenance check.
  - To suppress fixture noise at the source, add those paths to `[index] exclude` in
    `.coregraph/config.toml` and re-index.

## Where to look (references)

| Need | Reference |
|---|---|
| Every subcommand and flag | [`references/cli-reference.md`](references/cli-reference.md) |
| Step-by-step analysis of an unfamiliar repo, interpretation criteria | [`references/analysis-workflow.md`](references/analysis-workflow.md) |
| How an LLM should drive coregraph (the `--output-format llm` path, MCP fast-path) | [`references/llm-usage.md`](references/llm-usage.md) |
| Daemon races, PATH issues, empty queries, reset | [`references/troubleshooting.md`](references/troubleshooting.md) |

# Graph Model — Nodes, Edges, Trust & Confidence

CoreGraph stores your code as a directed graph of **symbol nodes** and **typed
edges**. Every edge carries enough metadata for an LLM (or a human) to know how
much to trust it: a *kind*, an *analysis origin*, a *trust model*, and a
*confidence score* that decays as the underlying source goes stale.

This page is the reference for that model. To see the live taxonomy for your own
repo, run:

```
coregraph stats --breakdown
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

The numbers above are this repository's own graph; yours will differ. The
*categories* — the four sections — are fixed and described below.

---

## 1. Node kinds

A node is one symbol. The kinds fall into four groups.

| Group | Kinds |
|---|---|
| Code constructs | `Function`, `Method`, `Class`, `Struct`, `Interface`, `Trait`, `Enum`, `EnumVariant`, `Constant`, `Variable`, `Field`, `TypeAlias`, `Module`, `Namespace` |
| Structural containers | `File` (a source file as a node; owns `Contains` edges to everything defined inside it) |
| Config & documentation | `ConfigKey` (a YAML/TOML/JSON path such as `spring.datasource.url`), `StringLiteral`, `DocComment`, `DocSection` |
| Packages | `ExternalPackage` (an npm / crates.io / maven dependency, minted from unresolved imports). `Package` (a first-party manifest unit) is a defined kind but is not yet emitted as a graph node — manifest parsing currently feeds only the orphan library/application classifier. |

Each node also records: a local `name`, a `qualified_name` for cross-file
identity, the source `file` and byte span, and a life-cycle `status`. The source
`language` is not stored on the node; it is derived on demand from the recorded
file path's extension.

| Status | Meaning |
|---|---|
| `Verified` | The source file was parsed and confirmed. |
| `Stale` | The source file changed; the node needs re-parsing. |
| `Assumed` | Inferred / cross-file fixup that was later undone; the node never had direct source evidence. |
| `Gone` | Deleted from source (rename/delete); removed by periodic GC. |

In practice, `Verified` and `Gone` are the statuses assigned during normal
operation. `Assumed` is currently never assigned in production, and node-level
`Stale` marking is effectively unreachable — staleness is tracked through edge
epochs and `stale_evidence_count` rather than a node status change.

---

## 2. Edge kinds

The graph stores **only direct (1-hop) edges**. Transitive paths are *not*
stored — they are computed at query time. This keeps the graph small and lets
`--hop-limit` / `--max-depth` decide how far traversal goes per query.

| Group | Kinds | Notes |
|---|---|---|
| Code relationships | `Calls`, `References`, `Imports` | Syntactic facts read from the source. |
| Type / contract | `Extends`, `Inherits`, `Implements`, `Overrides`, `TypeOf`, `GenericParam` | `TypeOf` = a variable/field/parameter declares a type; `GenericParam` = `List<T>` → `T`. |
| Name resolution | `Resolves` | Cross-file binding produced by stack-graphs. |
| Value / config | `StringMatch`, `EnumValueMatch`, `ApiPathMatch`, `Configures` | Literal and config matches. `Configures` carries framework-mediated bindings (DI, config keys, routes). |
| Structure | `Contains` (File → Symbol), `BelongsTo` (Symbol → Module/Namespace) | |
| Manifest | `DependsOn` | Package-level dependency. Defined kind; not yet inserted into the symbol graph (manifest `DependsOn` edges live only inside the manifest crate's parsed model). |
| Documentation | `Documents`, `Mentions`, `DescribedIn` | See §6. |

To filter a query to specific edge kinds, use `--edge-kind` (repeatable):

```
coregraph query compute_impact --direction incoming --edge-kind calls --hop-limit 1
```

`--edge-kind` currently accepts only these 10 values: `resolves`, `calls`,
`implements`, `extends`, `overrides`, `references`, `imports`, `string-match`,
`configures`, `depends-on`. The remaining kinds in the table above (`inherits`,
`type-of`, `generic-param`, `enum-value-match`, `api-path-match`, `contains`,
`belongs-to`, `documents`, `mentions`, `described-in`) cannot be filtered yet.

Each stored edge records its `kind`, the `origin` that produced it, the
`evidence_file` that grounds it, the graph epoch it was created at
(`created_at_epoch`), a stored `confidence`, and a `stale_evidence_count`. There
is no separate `mediator_file` field: for externally-mediated (`Configures`)
edges the mediator path is stored as the edge's sole `evidence_file`. The
*current* confidence is recomputed on read as `origin base_score × 0.7 ^
stale_evidence_count` (see §4).

---

## 3. Trust models

The simple rule "an edge's trust comes from its source file" only holds for a
few edge kinds. Which endpoints actually provide evidence depends on the
relationship. CoreGraph generalizes this into **four trust models**. The trust
model is surfaced as edge metadata in query/export/stats output and classifies
*which files would provide evidence* for an edge.

> **Note:** The per-trust-model re-parse scope below is the intended design and
> is encoded in code, but it is **not yet wired** into the healing pipeline. The
> healing path (on-demand and daemon pre-dispatch) currently re-extracts any
> graph file whose content hash changed within a time budget, regardless of
> trust model. The "Healing re-parses" column describes the planned mapping, not
> the operative selection.

| Trust model | Edge kinds | Healing re-parses (planned) | When it becomes uncertain |
|---|---|---|---|
| **SourceEvidenced** | `Calls`, `References`, `Imports`, `Resolves`, `Contains`, `BelongsTo`, `DependsOn`, `Documents`, `Mentions` | source file only | source goes stale → the edge's existence is in doubt |
| **ContractDependent** | `Extends`, `Inherits`, `Implements`, `Overrides`, `TypeOf`, `GenericParam` | source + target | target goes stale → contract fulfillment is in doubt |
| **Bidirectional** | `StringMatch`, `EnumValueMatch`, `ApiPathMatch`, `DescribedIn` | source + target | either side changes → the match is in doubt |
| **ExternallyMediated** | `Configures` | source + target + mediator | mediator (config/router/container) goes stale → the binding is in doubt |

### SourceEvidenced

```
A.java (✓) ──calls──> B.java (⚠ stale)
```

The fact that `A` calls `B.foo()` is observable from `A` alone, so the edge
stays valid. Whether `B.foo()`'s signature still matches is a separate
*semantic* question about the target node — `B` is marked stale, but the edge
remains. **Stale does not cascade:** when `B` goes stale, only edges authored
*in* `B` are invalidated; edges *into* `B` survive.

### ContractDependent

```
Child.java (✓) ──inherits──> Parent.java (⚠ stale)
```

The relationship *exists* because `Child` writes `extends Parent`. Whether the
contract is *fulfilled* (abstract members still implemented, types still match)
needs both files. CoreGraph tracks these as two separate concerns:
declaration (decided by source) and contract fulfillment (needs source +
target).

### Bidirectional

```
Java:   @RequestMapping("/api/v1/cards")   (✓)
React:  fetch("/api/v1/cards")             (⚠ stale)
```

Either side changing can break the match, so both must be verified for the edge
to be trusted.

### ExternallyMediated

```
UserController.java (✓) ──configures──> CardService.java (✓)
                              │
                    mediated by: beans.xml (⚠ stale)
```

Both code files are fine, but a *third* file decides the binding. If that
mediator is stale, the relationship is no longer trustworthy. CoreGraph detects
mediators per framework — see [confidence.md](confidence.md) and the
cross-language mediator support (Spring DI, Spring config, React Router, Docker
Compose, Go DI). Mediated edges are tagged `ConventionInferred` with stored
confidence 0.28 (below the default `min_confidence` of 0.70), so they are
filtered out of default query output. Names that are too short are conservatively
rejected (Spring DI requires ≥4 characters; Go DI requires provider names >3
characters). Multi-target ambiguity, however, is *not* resolved to a single
edge: Docker Compose fans `depends_on` out to every declared service (a coarse
upper bound), Go DI emits an all-pairs provider lattice, Spring DI emits an edge
to every matching class, and React Router links a route to every PascalCase
symbol within 200 bytes. These guessed edges are present in the graph but kept
below the 0.70 threshold rather than omitted.

---

## 4. Analysis origin & confidence

There is no separate "verification step". An edge's confidence is a function of
**how it was produced** (its origin) and **how fresh its evidence is** right
now — there is no extra trust gate.

### Analysis origins

The pipeline tags each edge with the layer that produced it. Five origins are
produced in practice, in descending trust:

- **`CompilerDerived`** — structurally certain.
- **`NameResolved`** — name binding resolved.
- **`SyntaxMatched`** — syntactic match only.
- **`PatternMatched`** — value / pattern inference (false positives possible).
- **`ConventionInferred`** — convention / config heuristic.

What produces each origin, its base score, and the stored/live confidence
formulas (`base(kind) × base(origin)`, decayed by stale evidence) are the
**single source of truth in [confidence.md](confidence.md)** — refer there
rather than duplicating the detail here.

### Inherent limits of static analysis

Some relationships can't be resolved with certainty from source alone. CoreGraph
does not pretend otherwise — it tags low-confidence origins and surfaces the
uncertainty rather than guessing:

| Case | Why it's hard | How CoreGraph handles it |
|---|---|---|
| Generated code (e.g. protobuf) | not present until the build runs | query output computes a render-time `generated` (bool) + `generator` string from file-path heuristics (e.g. `.pb.go` → `generated: true`, `generator: "protoc"`); this is not persisted on the node. Only path-detectable generators are recognized — Lombok and MapStruct have no detection yet. |
| Dynamic / partial string matches | `fetch(\`/api/v1/${entity}\`)` is ambiguous | `PatternMatched` + low confidence |
| Macros / metaprogramming | tree-sitter sees pre-expansion source only | per-pattern inference rules |

---

## 5. Impact risk scoring

`coregraph impact <SYMBOL> --risk` augments the reachable-symbol set with a
confidence-weighted blast-radius assessment:

```
coregraph impact build_router --risk
```

```
Impact of 'build_router': 1251 reachable symbols, 1251 edges, depth 3
  Risk Score: 0.96 (Critical)
  Blast Radius: Critical (16 modules, 910 callers)
  Confidence-Weighted Impact: 653.500
  Affected tests: 334
    test_app (distance 2, path_confidence 0.90) — crates/server/src/handlers.rs
    create_app_returns_router (distance 2, path_confidence 0.90) — crates/server/src/lib.rs
    ... (more affected tests)
```

### Blast radius

| Class | Threshold |
|---|---|
| Low | ≤ 2 modules and ≤ 5 callers |
| Medium | 3–5 modules or 6–20 callers |
| High | > 5 modules or > 20 callers |
| Critical | > 10 modules or > 50 callers |

### Risk score weights

The overall risk score is a weighted blend of four factors:

| Factor | Weight | Calculation |
|---|---|---|
| Visibility | 20% | public symbols score higher |
| Direct callers | 45% | caller count weighted by path confidence |
| Module spread | 25% | cross-module impact multiplied by confidence |
| Impact kind | 10% | breaking vs additive changes |

### Risk classification

| Risk score | Class |
|---|---|
| < 0.4 | Low |
| 0.4 – 0.6 | Medium |
| 0.6 – 0.8 | High |
| > 0.8 | Critical |

### Confidence-weighted impact

The differentiator is that impact is *not* a raw hop count. CoreGraph multiplies
the confidence of every edge along a path, so a long chain of low-confidence
edges contributes far less to the score than a short chain of compiler-derived
ones:

```
path_confidence(path) = ∏ confidence(edge)   for every edge on the path
```

The reported `Confidence-Weighted Impact` is the sum of those per-path
confidences across all impacted paths, and each affected test is reported with
its `distance` (hops) and `path_confidence`.

---

## 6. Documentation layer

CoreGraph treats the relationship between code and its documentation as a
first-class part of the same graph — not a separate index. It adds two node
kinds and three edge kinds.

| Node / edge | Direction | Meaning |
|---|---|---|
| `DocComment` (node) | — | A doc comment (`///`, `//!`, `/** */`, JSDoc, docstring) attached to a code symbol. Indexed as `doc::<symbol>`, so "find the doc for X" works. |
| `DocSection` (node) | — | A section of an external Markdown file (a heading and its body) that references at least one code symbol. Named `docsection::<heading>`. |
| `Documents` (edge) | DocComment → Symbol | The doc comment documents that symbol. SourceEvidenced (same file). |
| `Mentions` (edge) | DocComment → Symbol | The doc text links to a symbol via an intra-doc link. SourceEvidenced. |
| `DescribedIn` (edge) | Symbol → DocSection | A code symbol is described in an external Markdown section. Bidirectional. |

### Documents — attaching docs to symbols

A `Documents` edge is created only when a **dedicated** doc marker is immediately
adjacent to a definition (the language's own doc-attachment rule, not a
"nearest comment" heuristic). A blank line breaks the attachment.

| Language | Doc form | Origin / confidence |
|---|---|---|
| Rust | `///`, `//!`, `/** */`, `/*! */` (preceding sibling) | SyntaxMatched 0.85 |
| Java | Javadoc `/** */` (preceding sibling) | SyntaxMatched 0.85 |
| TypeScript / JavaScript | JSDoc `/** */` (preceding sibling, through `export` wrappers) | SyntaxMatched 0.85 |
| Python | docstring (first body string, PEP 257) | SyntaxMatched 0.85 |
| Go | any adjacent comment, `//` line block or `/* */` (godoc convention, no dedicated marker) | PatternMatched 0.60 |

Languages with a dedicated marker get 0.85. Go has no dedicated marker, so its
doc edges are convention-based at 0.60 — below the default `min_confidence`
(0.70), they only appear at a lower threshold (precision over recall).

### Mentions — intra-doc links

When a doc comment's text links to a symbol — markdown ``[`Name`]`` /
``[`mod::Name`]`` (rustdoc), or `{@link Name}` / `{@linkcode Name}` /
`{@linkplain Name}` / `{@link Foo#bar}` (JSDoc/Javadoc) — CoreGraph adds a
`Mentions` edge. Bare
`[name]` is *not* recognized (too easily confused with prose links). Resolution
is name-based and may cross files, but an edge is created **only when the name is
unique** (no scope information means ambiguity is silently skipped). Confidence
is `PatternMatched` (0.60). Mentions are **not impact-bearing** — a doc node
never enters a code blast radius; "find docs that mention X" is a reverse query,
and doc staleness on a mentioned symbol is a drift concern, not impact.

### DescribedIn — external Markdown

`.md` / `.markdown` files are collected (under the same ignore rules as code) and
split into sections at ATX headings (`#`…`######`; headings inside fenced code
blocks, delimited by ``` ``` ``` or `~~~`, are ignored). A `DocSection` node is created **only** for sections that
resolve at least one code symbol, avoiding noise. Inside a section, a single
backticked identifier ``` `Name` ``` that matches a **unique** code symbol
produces a `Symbol → DocSection` `DescribedIn` edge. Multi-word or non-identifier
spans (``` `git status` ```, ``` `a.b` ```) are excluded. Confidence is
`PatternMatched` (0.60); the trust model is `Bidirectional` (the symbol name and
the doc reference must agree); not impact-bearing.

### Doc drift — `inconsistencies --category doc-drift`

This is a detector, not new graph structure. In a single build it flags the
common case "the signature changed but the doc didn't": a `@param name`
(JSDoc/Javadoc) or `:param name:` (Python) that names a parameter the function no
longer has.

```
coregraph inconsistencies --category doc-drift
```

- **Precision-first.** Only pure-identifier parameters are checked (dotted
  `opts.foo`, varargs `...args` are skipped). The real parameter set is
  over-collected (every identifier in the parameter list, including
  destructured shorthand bindings like `{ a, b }` in TS/JS), so a binding the
  walker missed never produces a false drift report. Only functions that
  actually have parameters are reported.
- **Underscore renames are not drift.** A documented `name` whose signature
  counterpart is `_name` (a single leading underscore — the unused-parameter
  convention) is treated as in sync; a double underscore (`__name`) is a real
  mismatch.
- **Rename candidates.** When a documented name is genuinely absent, signature
  parameters within edit distance 2 are suggested in the report detail —
  "closest signature parameter: … (likely rename)" — and exposed as a
  `candidates` array per report in `--output-format json`.
- **Coverage:** Java / TypeScript / JavaScript (`@param`), Python (`:param`).
  Rust rustdoc (`# Arguments` prose) and Go (sentence-style docs) have no
  parameter-tag convention, so they are not checked.

---

[Back to README](README.md)

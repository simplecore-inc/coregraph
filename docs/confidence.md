# Edge Confidence Model

Every edge in the graph carries a confidence score on `[0, 1]`. You use it to
filter noise: pass `--min-confidence <0.0–1.0>` (default **0.70**) and CoreGraph
drops every edge below the threshold. The flag is a global option accepted on
every subcommand, but the edge-dropping filter is currently wired into `query`
and `export` only — `impact`, `diff`, `orphans`, and `inconsistencies` accept
the flag and ignore it (their output is unchanged across thresholds).

```
coregraph query compute_impact --min-confidence 0.90
```

Raise the threshold to keep only the most certain relations (structural and
compiler-derived); lower it to surface heuristic guesses. The rest of this page
is the single source of truth for how the score is computed, so you can pick a
threshold deliberately. (For *what* each origin and trust model means at the
model level, see [graph-model.md](graph-model.md).)

## How the score is computed

An edge actually carries **two** confidence numbers, computed from different
inputs:

```
confidence         = base(kind) × base(origin)                  (stored, clamped to [0,1])
current_confidence = base(origin) × 0.7 ^ stale_evidence_count  (live, recomputed on read)
```

- The **stored `confidence`** is the product of two baselines — one for the
  *kind* of relation, one for the *origin* of the analysis that produced it —
  clamped to `[0, 1]`. The product form means a stronger evidence source can't
  upgrade a weaker edge kind, and vice versa: a `PatternMatched` `Resolves` edge
  scores the same as a `NameResolved` `StringMatch`.
- The **live `current_confidence`** applies stale-evidence decay to the *origin
  base alone* and is recomputed on every read from the live stale state (see
  [Stale-evidence decay](#stale-evidence-decay)).

The two numbers have **different inputs**: the edge-kind factor is in the stored
`confidence` but *not* in `current_confidence`. So at zero staleness
`current_confidence` equals the origin base, which is **≥** the stored value —
equal only when the kind base is `1.0` (`Resolves` / `Contains` / `BelongsTo`),
and strictly greater for every kind whose base is below `1.0`.

## Per-kind baselines

| EdgeKind        | base  | rationale |
| --------------- | ----- | --------- |
| `Resolves`      | 1.00  | name-resolution backed by a real resolver |
| `Contains`      | 1.00  | structural, observable from the AST |
| `BelongsTo`     | 1.00  | inverse of `Contains` |
| `Calls`         | 0.90  | call site resolved to a definition |
| `Implements`    | 0.90  | Java/Kotlin `implements`, Rust `impl Trait` |
| `Extends`       | 0.90  | subclassing |
| `Inherits`      | 0.90  | language-specific strict inheritance |
| `Imports`       | 0.85  | import specifier resolved to a target |
| `TypeOf`        | 0.85  | parameter/return type annotation |
| `Overrides`     | 0.85  | method override in subclass |
| `References`    | 0.80  | any other identifier reference |
| `GenericParam`  | 0.80  | `List<T>` → `List` binding |
| `EnumValueMatch`| 0.75  | same string value across enum variants |
| `ApiPathMatch`  | 0.75  | similar HTTP path literals across files |
| `StringMatch`   | 0.70  | raw cross-file literal match |
| `Configures`    | 0.70  | mediator-inferred configuration link |
| `DependsOn`     | 0.60  | coarse package-level dependency |

The documentation-layer kinds (`Documents`, `Mentions`, `DescribedIn`) carry a
kind base of `1.00`; their level is set entirely by the origin — see
[graph-model.md §6](graph-model.md#6-documentation-layer).

## Per-origin baselines

| AnalysisOrigin       | base  | produced by |
| -------------------- | ----- | ----------- |
| `CompilerDerived`    | 0.99  | structurally-certain facts read directly from the syntax tree — `Contains` / `BelongsTo` (file → symbol containment, module membership) |
| `NameResolved`       | 0.95  | a name resolved to its definition — a same-file or same-directory match, or a stack-graphs cross-file stitch |
| `SyntaxMatched`      | 0.85  | a tree-sitter syntactic match (a call expression, a type annotation), or the syntactic-fallback resolver's globally-unique-name match when stack-graphs produced no binding |
| `PatternMatched`     | 0.60  | pattern / value inference — API-path regexes, intra-doc links (`{@link}`, `` [`X`] ``); false positives possible |
| `ConventionInferred` | 0.40  | convention- or config-derived: framework mediators (Spring DI/config, React Router, Docker Compose, Go DI) and naming conventions |

## Choosing a `--min-confidence`

The CLI default is **0.70**. Because the score is a *product*, a threshold acts
on `base(kind) × base(origin)`, not on the origin tier alone — a low-base kind
can land under the cutoff whatever its origin. Raising the threshold broadly
peels off layers:

- **0.70** (default): keeps syntactic-and-stronger edges — a `SyntaxMatched` × `Imports` edge (0.7225) survives — and drops `PatternMatched` / `ConventionInferred` guesses. Low-base kinds still fall under it regardless of origin: `DependsOn` edges (≤ 0.594) and mediator `Configures` edges (0.28) never clear the default.
- **0.85**: keeps high-base kinds at `NameResolved`/`CompilerDerived` grade; a `SyntaxMatched` `Resolves` edge still clears it (0.85 × 1.00 = 0.85) because `Resolves` has kind base 1.00.
- **0.90**: only structurally-resolved edges (`CompilerDerived` 0.99, plus `NameResolved` `Resolves` at 0.95). To leave only `CompilerDerived`, raise the threshold to ~0.96+.

The old default of 0.85 silently dropped every `Imports` edge on a
`SyntaxMatched` path (0.7225 < 0.85), which made cross-file TypeScript import
graphs invisible by default. That trap is why the default was lowered.

## Stale-evidence decay

Edges keep a `stale_evidence_count`. Each edge carries exactly one
`evidence_file` — the file whose extraction produced it. The count increments
during a fast single-file reindex of a file that contains one of the edge's
*endpoints*: a surviving cross-file edge whose `evidence_file` differs from the
reindexed path is re-linked to the file's newly-extracted symbols with
`stale_evidence_count + 1`, and the reported `current_confidence` shrinks. (When
the edge's own `evidence_file` changes, the edge is dropped and re-extracted —
fresh count `0` if re-observed, removed otherwise — so that case does not
increment the count.) A count above `1` therefore comes from repeated
endpoint-file reindexes, not from multiple stale evidence files. Decay
multiplies the *origin base* by `0.7` for each accumulated stale count:

```
current_confidence = base(origin) × 0.7 ^ stale_evidence_count
```

| `stale_evidence_count` | multiplier |
| --- | --- |
| 0 | × 1.00 (full origin base) |
| 1 | × 0.70 |
| 2 | × 0.49 |

Worked example: a `SyntaxMatched` `Calls` edge stores
`confidence = 0.90 × 0.85 = 0.765`. With a stale count of one its
`current_confidence` decays from the *origin* base: `0.85 × 0.7 = 0.595`. (Note
0.595 is the decayed `current_confidence`, not the stored `confidence` of 0.765,
and it decays the origin base 0.85 — distinct from the kind base 0.90.)

Two filters apply here, and they are **independent**:

- `--min-confidence` filters on the **stored** `confidence` (here 0.765), never
  on the decayed `current_confidence`. The decayed value surfaces only in JSON
  output and impact-risk weighting, so lowering the threshold does not reveal an
  epoch-stale edge, and a decay-affected edge whose stored `confidence` stays
  ≥ 0.70 remains visible by default.
- `--include-stale` bypasses a separate, epoch-based staleness gate: an edge is
  hidden when its `created_at_epoch` predates the graph's max epoch, regardless
  of its confidence. Passing the flag shows those epoch-stale edges.

### Seeing both numbers

In `json` output you can see the stored value and the live one side by side:

```json
{
  "direction": "incoming", "kind": "calls", "depth": 1,
  "other_name": "run",
  "confidence": 0.8549999594688416,
  "trust": "NameResolved", "origin": "NameResolved",
  "trust_model": "SourceEvidenced",
  "stale_evidence_count": 0, "current_confidence": 0.95
}
```

Here the stored `confidence` is `0.90 × 0.95 = 0.855` (kind `Calls` × origin
`NameResolved`), while the live `current_confidence` is `0.95` — the origin base
with no stale evidence. They differ precisely because `current_confidence` omits
the kind factor.

## Where each value is set

- Edge kind baselines: `crates/graph/src/edge_evaluator.rs::EdgeEvaluator::evaluate`
- Origin baselines: `crates/core/src/edge.rs::AnalysisOrigin::base_score`
- Default `--min-confidence`: `crates/cli/src/global_opts.rs` (also mirrored in `main.rs`)
- Current confidence after decay: `crates/core/src/edge.rs::DirectEdge::current_confidence`

---
[Back to index](README.md)

# CoreGraph end-to-end tests

Exercises the `coregraph` CLI binary against real input to verify observable
behavior — **not** a unit or integration test suite. Complements the Rust test
suite by catching regressions that only surface at the binary boundary.

## Layout

```
tests/e2e/
├── run.sh                # top-level entry
├── lib/
│   ├── common.sh         # run_cg, assert_eq, assert_range, ...
│   └── invariants.sh     # self-consistency checks (stats↔export, dangling edges)
├── golden/               # primary oracle — hand-crafted scenarios with known truth
│   ├── run.sh
│   ├── 01-ts-single-package/
│   ├── 02-ts-cross-package/
│   ├── 03-orphans/
│   ├── 04-inconsistencies/
│   └── 05-multi-lang/
└── tier2/                # secondary — real open-source repos, robustness only
    ├── run.sh
    ├── projects.toml     # SHA-pinned project list
    ├── cache/            # gitignored shallow clones
    └── snapshots/        # regression tracking
```

## Running

```bash
# Ensure binary is fresh
cargo build --workspace             # debug (golden only)
cargo build --workspace --release   # needed for tier 2 (large repos)

# Run everything
tests/e2e/run.sh

# Golden only (fast, no network)
tests/e2e/run.sh --golden-only

# Tier 2 only
tests/e2e/run.sh --tier2-only

# Build release + run everything
tests/e2e/run.sh --build
```

## What "pass" means

### Golden scenarios

Each scenario asserts **exact** expected output for a small, hand-written
fixture. Scenarios encode the contract: "given this input, the CLI must
produce these symbols, these edges, this query result." A failure here
means a regression in extraction or query correctness.

### Tier 2

Asserts only that the binary does not crash, produces non-zero output, and
stays within wide collapse-prevention ranges on real repos. Tier 2 does not
verify correctness — it verifies the tool survives being pointed at
unfamiliar code. Precision assertions belong in golden.

## Adding scenarios

1. Create `tests/e2e/golden/NN-<name>/` (use next free `NN`).
2. Add source files that exercise the CLI feature you want to cover.
3. Write `scenario.sh` that sources `../../lib/common.sh` and uses
   `run_cg`, `assert_eq`, `assert_jq`, etc. End with `print_summary`.
4. `chmod +x scenario.sh`, run it, iterate until all assertions pass.
5. `tests/e2e/golden/run.sh` picks up new scenarios automatically
   (glob `[0-9][0-9]-*/scenario.sh`).

## Adding tier 2 projects

1. Find a `sha` on the default branch.
2. Add a `[[project]]` block to `tests/e2e/tier2/projects.toml`.
3. Run once to generate a baseline snapshot and tune ranges.
4. Commit the snapshot.

## Known limitations (as of the most recent run)

Golden surfaced these, and they are tracked — the harness does not hide them:

- Query ranking still favours exact match > prefix > substring (shorter name
  first). Partial/fuzzy matches that beat every index key are rare but not
  impossible on very large graphs with naming collisions.

### Fixed in the latest pass

- **Edge explosion**: `stack::apply_resolutions_report` emitted a full
  `from_ids × to_ids` Cartesian product for each reference. Real repos hit
  100M+ edges (excalidraw: 122M). Now 1 edge per reference picks the most
  specific (smallest-span) node on each side → excalidraw drops to ~212k
  (**577× reduction**); petclinic: 8,371 → 1,703 (5×).
- **Cross-file TS imports**: `structural_pass` (which creates the File
  nodes) now runs **before** `resolve_references`. Top-of-file import
  specifiers attach to the File node via the fallback in `enclosing_symbol`
  and emit real `Imports` edges (11 on scenario 02 instead of 0).
- **Impact direction**: `compute_impact` now traverses both incoming and
  outgoing edges. `impact Product` reaches the 8 files/consumers that
  import or use it, instead of returning 0.
- **Query ranking**: `lookup_by_name_fuzzy` now ranks substring matches —
  exact case-insensitive match first, then prefix match, then shorter
  names. "Excalidraw" no longer returns a longer `ConfigKey` that
  happens to include the substring.
- **TS enum extraction + enum-mismatch**: TS `enum` declarations emit
  `Enum` nodes plus `EnumVariant` nodes named `EnumName.VariantName`. The
  detector groups variants by normalized local name directly instead of
  riding on `StringMatch` edges (which never existed for TS variants).
  Scenario 04 catches `Role.ADMIN` vs `Permission.ADMIN`.
- **`--min-confidence` silent no-op**: `export` completely ignored the
  flag. All three formats (dot/cypher/json-graph) now filter edges by
  `confidence ≥ min_confidence`. Scenario 01: 0.0 → 51 edges, 0.9 → 30.
- **`orphans --exclude-tests`**: not a bug — scenario 03 fixtures live
  under `tests/` so the test-path heuristic correctly drops them.
  Scenario now asserts the empty set is the right answer.
- **stats/export divergence**: was a daemon-cache staleness effect, not
  a data pipeline bug. Invariant now forces `--min-confidence 0.0` on
  both sides and kills any stale daemon before comparison.

## Design notes

- **No framework**: bash + `jq` + `awk`. A scenario script is readable
  top-to-bottom.
- **No project-specific logic inside the CLI library** (see CLAUDE.md):
  every heuristic lives in `tests/e2e/`, not in `crates/**`.
- **Ranges over exact counts** in tier 2 to survive churn in real repos.
- **SHA pins** so tier 2 results are reproducible across weeks.
- `export --format json-graph` is not used in tier 2 because real repos
  produce multi-GB output. Tier 2 uses `stats` + `query` instead.

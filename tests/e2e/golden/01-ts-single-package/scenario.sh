#!/usr/bin/env bash
# Scenario 01: single TypeScript package
# Covers: index (implicit), stats, query, export, impact, output-format json

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/01-ts-single-package"
info "fixture: $SCENARIO_DIR"
P="$SCENARIO_DIR"

# ---- stats ----
head2 "stats"
stats_out=$(run_cg "$P" stats --output-format json)
syms=$(echo "$stats_out" | awk -F': ' '/^symbols:/{print $2}' | tr -d ' ')
edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
# NOTE: stats.symbols currently counts references in addition to definitions,
# which makes it ~1.8x the export.nodes count. Tracked as a known inconsistency;
# the invariant check below will fail until stats/export agree.
assert_range "stats.symbols" "15" "80" "$syms"
assert_eq "stats.edges" "73" "$edges"

# ---- export json-graph ----
head2 "export json-graph"
exp_json=$(run_cg "$P" export --format json-graph)
# export respects the default --min-confidence 0.70, which drops the
# lowest-trust edges from the full graph. Use --min-confidence 0.0 to see the
# complete set (asserted just below).
assert_eq "export.nodes" "21" "$(echo "$exp_json" | jq '.nodes | length')"
# Default --min-confidence 0.70 filters the sub-0.70 edges (TypeOf and
# GenericParam, scored ~0.51) — 9 of them here (7 TypeOf + 2 GenericParam),
# leaving 64 of the 73-edge full set.
assert_eq "export.edges (default 0.70)" "64" "$(echo "$exp_json" | jq '.edges | length')"
full_edges=$(run_cg "$P" --min-confidence 0.0 export --format json-graph | jq '.edges | length')
assert_eq "export.edges (unfiltered)" "73" "$full_edges"

# expected symbol names present
for sym in User UserId UserRepository UserService UserController bootstrap makeUserService getUser listUsers createUser handleGet handleList handleCreate; do
    assert_jq "export contains $sym" "$exp_json" "[.nodes[] | .name] | any(. == \"$sym\")"
done

# no stray symbols
assert_jq "export: no NonExistent symbol" "$exp_json" '[.nodes[] | .name] | all(. != "NonExistent")'

# edge kinds present
for kind in BelongsTo Contains Calls Resolves; do
    assert_jq "export has edge-kind $kind" "$exp_json" "[.edges[] | .kind] | any(. == \"$kind\")"
done

# ---- export dot ----
head2 "export dot"
dot_out=$(run_cg "$P" export --format dot)
assert_contains "dot header" "digraph coregraph" "$dot_out"
assert_contains "dot has UserController" "UserController" "$dot_out"
assert_contains "dot has Class" "Class" "$dot_out"

# ---- query UserController ----
head2 "query UserController"
q_out=$(run_cg "$P" query UserController --output-format json)
assert_jq "query center name" "$q_out" '.center.name == "UserController"'
assert_jq "query center kind" "$q_out" '.center.kind == "Class"'
assert_jq "query has edges" "$q_out" '.edges | length > 0'
assert_jq "query center file ends with controller.ts" "$q_out" '.center.file | endswith("controller.ts")'

# ---- query UserService ----
head2 "query UserService"
q_svc=$(run_cg "$P" query UserService --output-format json)
assert_jq "query UserService kind" "$q_svc" '.center.kind == "Class"'

# ---- query nonexistent symbol ----
head2 "query missing symbol"
missing_out=$(run_cg "$P" query ThisSymbolDoesNotExist --output-format json 2>&1 || true)
# Current behavior: text message "No symbol found for '<name>'", exit 0.
# The check: tool must explicitly acknowledge the miss, not silently return partial data.
assert_contains "missing query acknowledged" "No symbol found" "$missing_out"

# ---- impact ----
# Note: impact follows only impact-bearing edge kinds (Calls, Resolves,
# Implements, etc.). Leaf types without outgoing calls return 0 reachable —
# that's correct. Use a function that actually calls into other symbols.
head2 "impact bootstrap"
imp=$(run_cg "$P" impact bootstrap --output-format json)
assert_jq "impact bootstrap reaches symbols" "$imp" '.reachable >= 2'
assert_jq "impact bootstrap has edges" "$imp" '.edges >= 2'

# ---- orphans (human output smoke check) ----
head2 "orphans"
# Human output always prints the "Orphan symbols (N): ..." header, even when
# N == 0 (this fixture has no orphans — every symbol is referenced). The JSON
# format is a structured object and is exercised in scenario 03 instead.
orph=$(run_cg "$P" orphans 2>&1 || true)
assert_contains "orphans header" "Orphan symbols" "$orph"

# ---- --fast preset ----
head2 "query --fast"
fast_out=$(run_cg "$P" query UserController --fast --output-format json)
assert_jq "--fast query runs" "$fast_out" '.center.name == "UserController"'

# ---- --min-confidence filter ----
head2 "--min-confidence 0.9"
hi_conf=$(run_cg "$P" export --format json-graph --min-confidence 0.9)
hi_edges=$(echo "$hi_conf" | jq '.edges | length')
all_edges=$(echo "$exp_json" | jq '.edges | length')
# The filter must be non-trivial: low-confidence edges exist in this fixture
# (SyntaxMatched resolves at ~0.85) and should be excluded at 0.9.
if [[ "$hi_edges" -lt "$all_edges" ]]; then
    pass "min-confidence 0.9 excluded $((all_edges - hi_edges)) low-confidence edges"
else
    fail "min-confidence 0.9 did not filter anything (hi=$hi_edges, all=$all_edges)"
fi

# ---- invariants ----
run_all_invariants "$P"

# ---- summary ----
print_summary

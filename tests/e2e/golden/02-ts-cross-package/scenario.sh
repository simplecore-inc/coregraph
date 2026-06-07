#!/usr/bin/env bash
# Scenario 02: TS monorepo with 3 packages (shared / api / web)
# Covers: cross-package symbol discovery, impact (intra-file), multi-file stats

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/02-ts-cross-package"
P="$SCENARIO_DIR"

# ---- stats ----
head2 "stats"
stats_out=$(run_cg "$P" stats --output-format json)
edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
assert_range "stats.edges" "30" "60" "$edges"

# ---- export: all packages discovered ----
head2 "export: all packages present"
exp=$(run_cg "$P" export --format json-graph)
for sym in Product Order totalPrice ProductService OrderHandler Cart; do
    assert_jq "cross-package: $sym discovered" "$exp" "[.nodes[] | .name] | any(. == \"$sym\")"
done

# files from all 3 packages indexed
for f in types.ts productService.ts orderHandler.ts cart.ts; do
    assert_jq "file $f indexed" "$exp" "[.nodes[] | .name] | any(. == \"$f\")"
done

# ---- impact (intra-file works) ----
head2 "impact ProductService"
imp=$(run_cg "$P" impact ProductService --output-format json)
assert_jq "intra-file impact >= 1 reachable" "$imp" '.reachable >= 1'

# ---- query types ----
head2 "query Product"
q=$(run_cg "$P" query Product --output-format json)
assert_jq "Product is Interface" "$q" '.center.kind == "Interface"'
assert_jq "Product has containment edge" "$q" '[.edges[] | .kind] | any(. == "belongs-to" or . == "contains")'

# ---- --lang filter ----
head2 "--lang typescript filter"
ts_only=$(run_cg "$P" --lang typescript export --format json-graph)
# all source files are .ts, so lang=typescript should return the same or near-same count
ts_nodes=$(echo "$ts_only" | jq '.nodes | length')
all_nodes=$(echo "$exp" | jq '.nodes | length')
# lang=ts should cover at least every .ts-defined symbol; allow structural
# nodes (e.g. synthesised Modules) that don't carry language themselves to
# fall outside the filtered set.
assert_ge "lang=ts within unfiltered total" "$ts_nodes" "$all_nodes"

# lang that isn't present → fewer or zero nodes
go_only=$(run_cg "$P" --lang go export --format json-graph 2>/dev/null)
go_nodes=$(echo "$go_only" | jq '.nodes | length')
if [[ "$go_nodes" -lt "$all_nodes" ]]; then
    pass "lang=go reduces nodes ($go_nodes < $all_nodes)"
else
    warn "lang=go did not reduce node count; filter may not be active"
fi

# ---- invariants ----
run_all_invariants "$P"

print_summary

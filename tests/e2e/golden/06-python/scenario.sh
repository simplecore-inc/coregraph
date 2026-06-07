#!/usr/bin/env bash
# Scenario 06: Python cross-file extraction + resolution
# Covers: Python extractor, Imports edges across modules, impact traversal

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/06-python"
P="$SCENARIO_DIR"

# ---- stats ----
head2 "stats"
stats_out=$(run_cg "$P" stats --output-format json)
edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
assert_range "stats.edges" "25" "60" "$edges"

# ---- symbols discovered ----
head2 "symbols"
exp=$(run_cg "$P" export --format json-graph)
for sym in User UserRepository UserService make_service register lookup; do
    assert_jq "python: $sym discovered" "$exp" "[.nodes[] | .name] | any(. == \"$sym\")"
done

# ---- cross-file Imports edges ----
head2 "Imports edges exist"
assert_jq "imports edges > 0" "$exp" '[.edges[] | select(.kind == "Imports")] | length > 0'

# ---- query ----
head2 "query UserService"
q=$(run_cg "$P" query UserService --output-format json)
assert_jq "UserService is Class" "$q" '.center.kind == "Class"'

# ---- impact ----
head2 "impact make_service"
imp=$(run_cg "$P" impact make_service --output-format json)
assert_jq "make_service reaches at least 2" "$imp" '.reachable >= 2'

# ---- invariants ----
run_all_invariants "$P"

print_summary

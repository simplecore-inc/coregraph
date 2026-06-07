#!/usr/bin/env bash
# Scenario 04: inconsistency detection
# Covers: inconsistencies (all, --category enum-mismatch/api-path/config-key)

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/04-inconsistencies"
P="$SCENARIO_DIR"

# ---- all categories ----
head2 "inconsistencies (all)"
all=$(run_cg "$P" inconsistencies --output-format json)
assert_jq "inconsistencies returns array" "$all" '(. | type) == "array"'
assert_jq "total >= 2 reports" "$all" '. | length >= 2'

# ---- api-path: exact semantic match expected ----
head2 "--category api-path"
api=$(run_cg "$P" inconsistencies --category api-path --output-format json)
assert_jq "api-path: 2 reports" "$api" '. | length == 2'
assert_jq "api-path: catches users near-miss" "$api" \
    '[.[] | .shared_value] | any(contains("/api/v1/user vs /api/v1/users"))'
assert_jq "api-path: catches cards near-miss" "$api" \
    '[.[] | .shared_value] | any(contains("/api/v1/card vs /api/v1/cards"))'
assert_jq "api-path: server and client files both present" "$api" \
    '[.[] | [.a.file, .b.file] | .[]] | any(endswith("api-server.ts")) and any(endswith("api-client.ts"))'

# ---- config-key ----
head2 "--category config-key"
cfg=$(run_cg "$P" inconsistencies --category config-key --output-format json)
assert_jq "config-key: returns array" "$cfg" '(. | type) == "array"'
assert_jq "config-key: at least 1 report" "$cfg" '. | length >= 1'

# ---- enum-mismatch ----
head2 "--category enum-mismatch"
em=$(run_cg "$P" inconsistencies --category enum-mismatch --output-format json)
assert_jq "enum-mismatch: returns array" "$em" '(. | type) == "array"'
assert_jq "enum-mismatch: catches Role.ADMIN vs Permission.ADMIN" "$em" \
    '[.[] | .shared_value] | any(. == "admin")'
assert_jq "enum-mismatch: pairs span both files" "$em" \
    '[.[] | [.a.file, .b.file] | .[]] | any(endswith("roles.ts")) and any(endswith("permissions.ts"))'

# ---- unrelated symbol should NOT be flagged ----
head2 "negative: no inconsistencies involve 'isAdmin'"
assert_jq "isAdmin not in reports" "$all" \
    '[.[] | [.a.name, .b.name] | .[]] | all(. != "isAdmin")'

# ---- invariants ----
run_all_invariants "$P"

print_summary

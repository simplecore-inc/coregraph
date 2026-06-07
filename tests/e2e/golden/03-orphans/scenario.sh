#!/usr/bin/env bash
# Scenario 03: orphan detection with known expected set
# Covers: orphans, orphans --public-only, orphans --exclude-tests

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/03-orphans"
P="$SCENARIO_DIR"

# ---- basic orphans ----
head2 "orphans (default)"
orph=$(run_cg "$P" orphans --output-format json)
# JSON shape: {count, library_api_surface, test_code, likely_dead,
#              orphans:[{name, kind, file, line, external_api, is_test}]}
names=$(echo "$orph" | jq -r '.orphans[].name' | sort)
expected="divide
farewell
modulo"
assert_eq "orphan set" "$expected" "$names"

count=$(echo "$orph" | jq '.count')
assert_eq "orphan count" "3" "$count"

# ---- --public-only (same result here; all orphans are exported) ----
head2 "orphans --public-only"
pub_orph=$(run_cg "$P" orphans --public-only --output-format json)
pub_names=$(echo "$pub_orph" | jq -r '.orphans[].name' | sort)
assert_eq "public-only orphan set" "$expected" "$pub_names"

# ---- --exclude-tests ----
# The fixture's sources live in src/ and NONE are test files relative to the
# project root (-C points at this scenario dir). The test-path classifier is
# anchored to the project root, so an unrelated `tests/` ancestor in the
# absolute path is deliberately not treated as a test marker (no project-
# specific path hardcoding). So --exclude-tests drops nothing here and returns
# the same three orphans, with a test_code count of zero.
head2 "orphans --exclude-tests"
ex_orph=$(run_cg "$P" orphans --exclude-tests --output-format json)
ex_names=$(echo "$ex_orph" | jq -r '.orphans[].name' | sort)
assert_eq "exclude-tests keeps the non-test orphans" "$expected" "$ex_names"
assert_eq "exclude-tests test_code count is zero" "0" "$(echo "$ex_orph" | jq '.test_code')"

# ---- absence checks ----
head2 "non-orphans should NOT appear"
for sym in add multiply greet run; do
    assert_jq "non-orphan $sym absent" "$orph" "[.orphans[].name] | all(. != \"$sym\")"
done

# ---- invariants ----
run_all_invariants "$P"

print_summary

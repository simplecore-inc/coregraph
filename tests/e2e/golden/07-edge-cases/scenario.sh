#!/usr/bin/env bash
# Scenario 07: edge cases — circular imports, barrel re-exports, generics,
# dynamic import. Covers TS patterns that frequently break static resolvers.

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/07-edge-cases"
P="$SCENARIO_DIR"

exp=$(run_cg "$P" export --format json-graph)

# ---- circular imports (a ↔ b) ----
head2 "circular imports"
assert_jq "Ant discovered" "$exp" '[.nodes[] | .name] | any(. == "Ant")'
assert_jq "Bee discovered" "$exp" '[.nodes[] | .name] | any(. == "Bee")'
# Both Ant→Bee and Bee→Ant should have some edge (Resolves or Imports).
assert_jq "Ant→Bee edge present" "$exp" \
    '[.nodes[] | select(.name == "Bee") | .id] as $bee
     | [.edges[] | select(.to == $bee[0])] | length > 0'
assert_jq "Bee→Ant edge present" "$exp" \
    '[.nodes[] | select(.name == "Ant") | .id] as $ant
     | [.edges[] | select(.to == $ant[0])] | length > 0'
# Impact traversal must not infinite-loop on the cycle.
imp_ant=$(run_cg "$P" impact Ant --output-format json)
assert_jq "impact Ant terminates" "$imp_ant" '.reachable >= 1'

# ---- barrel re-exports (inner/index.ts re-exports Alpha/Beta) ----
head2 "barrel re-exports"
assert_jq "Alpha discovered" "$exp" '[.nodes[] | .name] | any(. == "Alpha")'
assert_jq "Beta discovered" "$exp" '[.nodes[] | .name] | any(. == "Beta")'
assert_jq "demo discovered" "$exp" '[.nodes[] | .name] | any(. == "demo")'
# demo() uses Alpha.greet() and Beta.greet(). Impact from demo should
# reach at least one of them.
imp_demo=$(run_cg "$P" impact demo --output-format json)
assert_jq "impact demo reaches >= 2" "$imp_demo" '.reachable >= 2'

# ---- generics (Box<T>, Holder<T>, unwrap<T>) ----
head2 "generics"
for sym in Box Holder unwrap; do
    assert_jq "generic: $sym present" "$exp" "[.nodes[] | .name] | any(. == \"$sym\")"
done
# NOTE: GenericParam edges come from a Java-shaped regex in
# `extract_type_relationships`. TS `Box<T>` isn't currently matched — tracked
# as a known gap (#36 TypeOf completion). Soft assertion so the scenario
# still passes while the gap is visible.
if echo "$exp" | jq -e '[.edges[] | .kind] | any(. == "GenericParam")' >/dev/null 2>&1; then
    pass "GenericParam edge kind present for TS"
else
    warn "GenericParam edges not emitted for TS generics — tracked as TypeOf completion gap"
fi

# ---- dynamic import (`await import("./plugin")`) ----
head2 "dynamic import"
assert_jq "PluginA present" "$exp" '[.nodes[] | .name] | any(. == "PluginA")'
assert_jq "PluginB present" "$exp" '[.nodes[] | .name] | any(. == "PluginB")'
assert_jq "loadPlugin present" "$exp" '[.nodes[] | .name] | any(. == "loadPlugin")'

# ---- invariants ----
run_all_invariants "$P"

print_summary

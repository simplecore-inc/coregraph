#!/usr/bin/env bash
# Scenario 05: multi-language project (TS + Rust + Go)
# Covers: stats --breakdown, --lang filter, cross-language extraction

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"
source "$SCENARIO_DIR/../../lib/invariants.sh"

ensure_bin
head1 "golden/05-multi-lang"
P="$SCENARIO_DIR"

# ---- stats ----
head2 "stats"
stats_out=$(run_cg "$P" stats --output-format json)
edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
assert_range "stats.edges" "20" "45" "$edges"

# ---- stats --breakdown ----
head2 "stats --breakdown"
bd=$(run_cg "$P" stats --breakdown --output-format json)
# breakdown output is mixed text; top line is JSON {"files": N, "symbols": N, "edges": N}
first_line=$(echo "$bd" | head -1)
files=$(echo "$first_line" | jq '.files')
syms_bd=$(echo "$first_line" | jq '.symbols')
assert_range "breakdown.files" "3" "10" "$files"
assert_ge "breakdown.symbols" "12" "$syms_bd"

# breakdown should mention each symbol kind we know to be present
for kind in Class Interface Struct Function Method Module; do
    assert_contains "breakdown mentions $kind" "$kind" "$bd"
done

# ---- symbols from each language present ----
head2 "symbol discovery per language"
exp=$(run_cg "$P" export --format json-graph)
# TS
assert_jq "TS: ApiClient present" "$exp" '[.nodes[] | .name] | any(. == "ApiClient")'
assert_jq "TS: healthCheck present" "$exp" '[.nodes[] | .name] | any(. == "healthCheck")'
# Rust
assert_jq "Rust: Server present" "$exp" '[.nodes[] | .name] | any(. == "Server")'
assert_jq "Rust: make_server present" "$exp" '[.nodes[] | .name] | any(. == "make_server")'
# Go
assert_jq "Go: Handler present" "$exp" '[.nodes[] | .name] | any(. == "Handler")'
assert_jq "Go: NewHandler present" "$exp" '[.nodes[] | .name] | any(. == "NewHandler")'

# ---- --lang filter ----
head2 "--lang filter"
ts_nodes=$(run_cg "$P" --lang typescript export --format json-graph | jq '.nodes | length')
rs_nodes=$(run_cg "$P" --lang rust export --format json-graph | jq '.nodes | length')
go_nodes=$(run_cg "$P" --lang go export --format json-graph | jq '.nodes | length')
all_nodes=$(echo "$exp" | jq '.nodes | length')

assert_ge "ts nodes >= 3" "3" "$ts_nodes"
assert_ge "rust nodes >= 3" "3" "$rs_nodes"
assert_ge "go nodes >= 3" "3" "$go_nodes"

# Sum of lang-filtered nodes should cover all language-tagged symbols; a
# small overhead of structural nodes (synthetic Module without a language
# file) may push the unfiltered total higher by 0-2.
sum=$((ts_nodes + rs_nodes + go_nodes))
diff=$((all_nodes - sum))
if [[ $diff -ge 0 && $diff -le 2 ]]; then
    pass "lang filter partition (sum=$sum, all=$all_nodes, diff=$diff ≤ 2)"
else
    fail "lang filter partition: unexpected diff $diff (sum=$sum, all=$all_nodes)"
fi

# ---- --lang filter returns only symbols from that language ----
head2 "--lang typescript returns only .ts files"
ts_only=$(run_cg "$P" --lang typescript export --format json-graph)
assert_jq "all ts files end with .ts" "$ts_only" \
    '[.nodes[] | select(.kind == "File") | .file] | all(endswith(".ts"))'

head2 "--lang rust returns only .rs files"
rs_only=$(run_cg "$P" --lang rust export --format json-graph)
assert_jq "all rust files end with .rs" "$rs_only" \
    '[.nodes[] | select(.kind == "File") | .file] | all(endswith(".rs"))'

head2 "--lang go returns only .go files"
go_only=$(run_cg "$P" --lang go export --format json-graph)
assert_jq "all go files end with .go" "$go_only" \
    '[.nodes[] | select(.kind == "File") | .file] | all(endswith(".go"))'

# ---- invariants ----
run_all_invariants "$P"

print_summary

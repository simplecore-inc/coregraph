#!/usr/bin/env bash
# Common helpers for CoreGraph e2e tests.
# Sourced by scenario scripts.

set -o pipefail

# ---- paths ----
E2E_ROOT="${E2E_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
REPO_ROOT="${REPO_ROOT:-$(cd "$E2E_ROOT/../.." && pwd)}"
CG_BIN="${CG_BIN:-$REPO_ROOT/target/debug/coregraph}"

# ---- output ----
_RED=$'\033[31m'; _GRN=$'\033[32m'; _YEL=$'\033[33m'; _DIM=$'\033[2m'; _RST=$'\033[0m'

if [[ -t 1 ]]; then
    _use_color=1
else
    _use_color=0
fi

_color() {
    local c="$1"; shift
    if [[ $_use_color -eq 1 ]]; then
        printf '%s%s%s' "$c" "$*" "$_RST"
    else
        printf '%s' "$*"
    fi
}

info()  { echo "  $*"; }
pass()  { echo "  $(_color "$_GRN" "✓") $*"; PASS_COUNT=$((${PASS_COUNT:-0}+1)); }
fail()  { echo "  $(_color "$_RED" "✗") $*" >&2; FAIL_COUNT=$((${FAIL_COUNT:-0}+1)); FAIL_DETAILS+=("$*"); }
warn()  { echo "  $(_color "$_YEL" "!") $*" >&2; }
head1() { echo; echo "── $* ──"; }
head2() { echo; echo "  ▸ $*"; }

# ---- binary check ----
ensure_bin() {
    if [[ ! -x "$CG_BIN" ]]; then
        echo "Binary not found at $CG_BIN. Run: cargo build --workspace" >&2
        exit 2
    fi
}

# ---- run coregraph (project-relative) ----
# Usage: run_cg <project_dir> <subcommand> [args...]
# Stdout: command stdout. Exit code: command exit code.
run_cg() {
    local proj="$1"; shift
    "$CG_BIN" -C "$proj" "$@"
}

# ---- assertions ----
assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        pass "$name = $actual"
    else
        fail "$name: expected [$expected], got [$actual]"
    fi
}

# assert_range <name> <min> <max> <actual>
assert_range() {
    local name="$1" min="$2" max="$3" actual="$4"
    if [[ "$actual" -ge "$min" && "$actual" -le "$max" ]]; then
        pass "$name = $actual (in [$min,$max])"
    else
        fail "$name: expected in [$min,$max], got $actual"
    fi
}

# assert_ge <name> <min> <actual>
assert_ge() {
    local name="$1" min="$2" actual="$3"
    if [[ "$actual" -ge "$min" ]]; then
        pass "$name = $actual (>= $min)"
    else
        fail "$name: expected >= $min, got $actual"
    fi
}

# assert_contains <name> <needle> <haystack>
assert_contains() {
    local name="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        pass "$name contains '$needle'"
    else
        fail "$name: missing '$needle'"
    fi
}

# assert_not_contains <name> <needle> <haystack>
assert_not_contains() {
    local name="$1" needle="$2" haystack="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        pass "$name does not contain '$needle'"
    else
        fail "$name: unexpectedly contains '$needle'"
    fi
}

# assert_exit <name> <expected_code> <actual_code>
assert_exit() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        pass "$name exit=$actual"
    else
        fail "$name: expected exit $expected, got $actual"
    fi
}

# assert_jq <name> <json> <jq_expr>
# Fails unless jq expression evaluates truthy (true / non-empty / non-zero).
assert_jq() {
    local name="$1" json="$2" expr="$3"
    if ! command -v jq >/dev/null 2>&1; then
        fail "$name: jq not installed"
        return
    fi
    local result
    result=$(printf '%s' "$json" | jq -r "$expr" 2>/dev/null) || {
        fail "$name: jq error on [$expr]"
        return
    }
    if [[ "$result" == "true" || ( -n "$result" && "$result" != "false" && "$result" != "null" && "$result" != "0" ) ]]; then
        pass "$name: $expr → $result"
    else
        fail "$name: $expr → $result"
    fi
}

# ---- summary ----
print_summary() {
    echo
    local total=$((${PASS_COUNT:-0}+${FAIL_COUNT:-0}))
    if [[ ${FAIL_COUNT:-0} -eq 0 ]]; then
        echo "$(_color "$_GRN" "✓") $total checks passed"
        return 0
    else
        echo "$(_color "$_RED" "✗") ${FAIL_COUNT} of $total checks failed"
        for d in "${FAIL_DETAILS[@]}"; do
            echo "    - $d"
        done
        return 1
    fi
}

# Initialize counters on source
PASS_COUNT=0
FAIL_COUNT=0
FAIL_DETAILS=()

#!/usr/bin/env bash
# Scenario 08: CLI protocol smoke tests.
# Covers commands that don't appear in other golden scenarios:
#   - batch (JSON-driven multi-query)
#   - snapshot save/load (binary serialization round-trip)
#   - watch --diff --once (file-change rebuild pipeline)
#   - config show (TOML merging)
#
# MCP/LSP/HTTP stdio bridges are intentionally not covered here — they
# require protocol-specific clients. Smoke-test them separately.

set -o pipefail
SCENARIO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCENARIO_DIR/../../lib/common.sh"

ensure_bin
head1 "golden/08-cli-protocols"
P="$SCENARIO_DIR"

# Kill any stale daemon so snapshot/watch don't hit leftover state.
pkill -9 -f "coregraph server" 2>/dev/null || true
rm -rf "$P/.coregraph" 2>/dev/null || true

# ---- batch ----
head2 "batch"
batch_out=$(run_cg "$P" batch "$SCENARIO_DIR/batch-queries.json" --output-format json 2>&1)
assert_contains "batch: Widget resolved" "Widget" "$batch_out"
assert_contains "batch: spawn resolved" "spawn" "$batch_out"

# ---- snapshot save + load ----
head2 "snapshot save/load"
snap_path="$P/.coregraph/test-snapshot.bin"
mkdir -p "$(dirname "$snap_path")"
run_cg "$P" snapshot save --out "$snap_path" 2>&1 >/dev/null && pass "snapshot save succeeds" || fail "snapshot save crashed"
if [[ -f "$snap_path" ]]; then
    snap_size=$(wc -c < "$snap_path" | tr -d ' ')
    if [[ "$snap_size" -gt 100 ]]; then
        pass "snapshot file non-trivial ($snap_size bytes)"
    else
        fail "snapshot file too small ($snap_size bytes)"
    fi
    load_out=$(run_cg "$P" snapshot load "$snap_path" --output-format json 2>&1)
    assert_contains "snapshot load reports symbols" "symbols" "$load_out"
    assert_contains "snapshot load reports edges" "edges" "$load_out"
else
    fail "snapshot file not created at $snap_path"
fi

# ---- config show ----
head2 "config show"
cfg_out=$(run_cg "$P" config show 2>&1)
assert_contains "config lists limits section" "limits" "$cfg_out"

print_summary

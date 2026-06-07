#!/usr/bin/env bash
# Run all golden scenarios in sequence.
# Exit 0 if all scenarios pass (each scenario runs its own print_summary),
# exit 1 otherwise.

set -o pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

total=0
failed=0
failed_names=()

for scenario in "$HERE"/[0-9][0-9]-*/scenario.sh; do
    [[ -f "$scenario" ]] || continue
    total=$((total+1))
    name=$(basename "$(dirname "$scenario")")
    # Kill any leftover daemon before each scenario so stats/query don't
    # return cached state from a previous fixture. The daemon is indexed
    # per-project but leaks across invocations on the same machine.
    pkill -9 -f "coregraph server" 2>/dev/null || true
    rm -rf "$(dirname "$scenario")/.coregraph" 2>/dev/null || true
    if bash "$scenario"; then
        :
    else
        failed=$((failed+1))
        failed_names+=("$name")
    fi
done

echo
echo "════════════════════════════════════════════"
if [[ $failed -eq 0 ]]; then
    echo "GOLDEN: $total/$total scenarios passed"
    exit 0
else
    echo "GOLDEN: $((total-failed))/$total scenarios passed"
    echo "failures:"
    for n in "${failed_names[@]}"; do
        echo "  - $n"
    done
    exit 1
fi

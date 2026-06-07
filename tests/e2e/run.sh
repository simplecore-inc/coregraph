#!/usr/bin/env bash
# Top-level CoreGraph e2e runner.
#
# Runs both the golden scenarios (primary oracle) and tier 2 real-world
# robustness suite (secondary).
#
# Flags:
#   --golden-only   Skip tier 2 (fast, no network)
#   --tier2-only    Skip golden
#   --build         Build workspace in release mode first
#
# Exit code 0 if all selected suites pass, 1 otherwise.

set -o pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

run_golden=1
run_tier2=1
do_build=0
for arg in "$@"; do
    case "$arg" in
        --golden-only) run_tier2=0 ;;
        --tier2-only)  run_golden=0 ;;
        --build)       do_build=1 ;;
        -h|--help)
            sed -n '2,14p' "$0"; exit 0 ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

if [[ $do_build -eq 1 ]]; then
    echo "▸ cargo build --workspace --release"
    (cd "$REPO_ROOT" && cargo build --workspace --release) || exit 2
fi

rc=0
if [[ $run_golden -eq 1 ]]; then
    bash "$HERE/golden/run.sh" || rc=1
fi
if [[ $run_tier2 -eq 1 ]]; then
    bash "$HERE/tier2/run.sh" || rc=1
fi

exit $rc

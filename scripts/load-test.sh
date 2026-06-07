#!/usr/bin/env bash
# Realistic load test for `coregraph index` against a large external
# project. Clones (or reuses) the target repo, runs index 3 times,
# captures wall time + RSS + final graph stats.
#
# Usage:
#   scripts/load-test.sh                          # rust-lang/cargo (default, ~50k LoC)
#   scripts/load-test.sh https://github.com/...   # any git URL
#
# Output: scripts/load-test-results/<repo-name>/{run1,run2,run3}.txt + summary.csv

set -euo pipefail

REPO_URL="${1:-https://github.com/rust-lang/cargo.git}"
REPO_NAME="$(basename "$REPO_URL" .git)"
SCRATCH_DIR="${SCRATCH_DIR:-/tmp/coregraph-load-test}"
TARGET_DIR="$SCRATCH_DIR/$REPO_NAME"
RESULTS_DIR="$(dirname "$0")/load-test-results/$REPO_NAME"

mkdir -p "$SCRATCH_DIR" "$RESULTS_DIR"

# Reuse a prior clone — running the script twice in a row should be
# fast on the network side. `git fetch` is enough to pick up new
# commits without re-downloading.
if [ -d "$TARGET_DIR/.git" ]; then
    echo "==> Updating existing clone in $TARGET_DIR"
    (cd "$TARGET_DIR" && git fetch --depth=1 origin)
else
    echo "==> Cloning $REPO_URL → $TARGET_DIR (shallow)"
    git clone --depth=1 "$REPO_URL" "$TARGET_DIR"
fi

# Always rebuild the CLI to guarantee we're measuring the current code.
echo "==> Building coregraph (release)"
cargo build --release --bin coregraph --quiet

CLI="$(pwd)/target/release/coregraph"

# Run 3 iterations, freshly clearing the .coregraph dir each time so
# every run measures cold-start cost.
SUMMARY="$RESULTS_DIR/summary.csv"
echo "run,wall_seconds,max_rss_kb,files,symbols,edges" > "$SUMMARY"
for i in 1 2 3; do
    rm -rf "$TARGET_DIR/.coregraph"
    OUT_FILE="$RESULTS_DIR/run${i}.txt"
    echo "==> Run $i — capturing to $OUT_FILE"
    # GNU /usr/bin/time gives RSS; macOS uses gtime if installed,
    # otherwise we fall back to bash's `time` (wall only) and skip RSS.
    if command -v gtime >/dev/null; then
        TIMER=(gtime -f '%e %M')
    elif /usr/bin/time -f '%e %M' true >/dev/null 2>&1; then
        TIMER=(/usr/bin/time -f '%e %M')
    else
        TIMER=()
    fi

    if [ ${#TIMER[@]} -gt 0 ]; then
        TIMINGS=$( { "${TIMER[@]}" "$CLI" -C "$TARGET_DIR" index --stats >"$OUT_FILE"; } 2>&1 | tail -1 )
        WALL=$(awk '{print $1}' <<< "$TIMINGS")
        RSS=$(awk '{print $2}' <<< "$TIMINGS")
    else
        START=$(date +%s)
        "$CLI" -C "$TARGET_DIR" index --stats >"$OUT_FILE"
        WALL=$(( $(date +%s) - START ))
        RSS="n/a"
    fi
    # Parse the index summary line:
    #   Index complete — N files, M symbols, K edges (...)
    LINE=$(grep -E "^Index complete" "$OUT_FILE" | tail -1)
    FILES=$(sed -E 's/.* — ([0-9]+) files.*/\1/' <<< "$LINE")
    SYMS=$(sed -E 's/.*, ([0-9]+) symbols.*/\1/' <<< "$LINE")
    EDGES=$(sed -E 's/.*, ([0-9]+) edges.*/\1/' <<< "$LINE")
    echo "$i,$WALL,$RSS,$FILES,$SYMS,$EDGES" >> "$SUMMARY"
    echo "    wall=${WALL}s rss=${RSS}kb files=$FILES symbols=$SYMS edges=$EDGES"
done

echo
echo "==> Summary"
column -t -s, < "$SUMMARY"

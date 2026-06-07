#!/usr/bin/env bash
# Tier 2: real-world robustness smoke test.
#
# Goal: verify the binary does not crash, produces *some* output, and stays
# within coarse ranges on large open-source projects. Correctness assertions
# live in the golden scenarios; tier 2 is only about collapse-prevention.
#
# For each project in projects.toml:
#   1. Ensure shallow clone at the pinned SHA (cached in cache/)
#   2. Run stats (primary), query <well-known>, orphans --public-only
#   3. Assert exit code 0, stats symbols/edges non-zero and within range,
#      wall-clock within limit
#   4. Write snapshots/<project>-<sha>.json for regression tracking
#
# We deliberately avoid `export --format json-graph` in tier 2 because on
# real codebases it can emit 100M+ edges and balloon to GB-scale output.

set -o pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E2E_ROOT="$HERE/.."
source "$E2E_ROOT/lib/common.sh"

ensure_bin
mkdir -p "$HERE/cache" "$HERE/snapshots"

if [[ -x "$REPO_ROOT/target/release/coregraph" ]]; then
    CG_BIN="$REPO_ROOT/target/release/coregraph"
    info "using release binary for tier 2"
else
    warn "release binary not found — tier 2 will be very slow with debug build"
fi

# ---- tiny TOML-like parser ----
parse_projects() {
    awk '
        /^\[\[project\]\]/ { if (n) print_row(); n++; for (k in v) delete v[k]; next }
        /^[a-zA-Z_]+[[:space:]]*=/ {
            key = $1
            sub(/[[:space:]]*=.*/, "", key)
            val = $0
            sub(/^[^=]+=[[:space:]]*/, "", val)
            gsub(/^"|"$/, "", val)
            v[key] = val
        }
        END { if (n) print_row() }
        function print_row() {
            printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
                v["name"], v["url"], v["sha"],
                v["min_symbols"], v["max_symbols"],
                v["min_edges"], v["max_edges"],
                v["max_wall_clock_sec"],
                v["well_known_symbols"]
        }
    ' "$HERE/projects.toml"
}

ensure_clone() {
    local name="$1" url="$2" sha="$3"
    local dir="$HERE/cache/$name"

    if [[ -d "$dir/.git" ]]; then
        local cur
        cur=$(git -C "$dir" rev-parse HEAD 2>/dev/null || echo "")
        if [[ "$cur" == "$sha" ]]; then
            info "cache hit: $name @ ${sha:0:12}"
            return 0
        fi
        info "cache miss (sha drift): refreshing $name"
        rm -rf "$dir"
    fi

    info "cloning $name"
    if ! git clone --quiet --filter=blob:none "$url" "$dir" >/dev/null 2>&1; then
        fail "$name: git clone failed"
        return 1
    fi
    if ! git -C "$dir" checkout --quiet "$sha" 2>/dev/null; then
        if git -C "$dir" fetch --quiet --depth=1 origin "$sha" 2>/dev/null && \
           git -C "$dir" checkout --quiet "$sha" 2>/dev/null; then
            :
        else
            fail "$name: cannot checkout $sha"
            return 1
        fi
    fi
    info "checked out $name @ ${sha:0:12}"
}

run_project() {
    local line="$1"
    IFS=$'\t' read -r name url sha min_sym max_sym min_edges max_edges max_sec wks <<< "$line"

    head1 "tier2: $name"
    info "sha: ${sha:0:12}"

    local dir="$HERE/cache/$name"
    ensure_clone "$name" "$url" "$sha" || return 1

    # ---- stats with wall-clock measurement ----
    head2 "stats"
    local t0 t1 elapsed stats_rc stats_out stats_err
    # Separate stderr — diagnostics (e.g. "skipped N minified/generated
    # file(s)", daemon banners) go to stderr and must not pollute the stdout
    # JSON parse below. Same lesson the query block already applies.
    stats_err=$(mktemp)
    t0=$(date +%s)
    stats_out=$(run_cg "$dir" stats --output-format json 2>"$stats_err")
    stats_rc=$?
    t1=$(date +%s)
    elapsed=$((t1 - t0))

    if [[ "$stats_rc" -ne 0 ]]; then
        info "stats stderr: $(head -c 300 "$stats_err")"
    fi
    rm -f "$stats_err"

    assert_exit "$name stats" "0" "$stats_rc"

    # Parse stats output. Newer versions may emit JSON `{"files": N, "symbols": N, "edges": N}`
    # while older paths emit plain "symbols: N\nedges: N". Handle both.
    local symbols edges
    if echo "$stats_out" | head -1 | grep -q '^{.*"edges"'; then
        symbols=$(echo "$stats_out" | head -1 | jq '.symbols')
        edges=$(echo "$stats_out" | head -1 | jq '.edges')
    else
        symbols=$(echo "$stats_out" | awk -F': ' '/^symbols:/{print $2}' | tr -d ' ')
        edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
    fi
    info "stats: symbols=$symbols edges=$edges wall=${elapsed}s"

    if [[ -n "$max_sec" ]]; then
        if [[ "$elapsed" -le "$max_sec" ]]; then
            pass "wall-clock <= ${max_sec}s (got ${elapsed}s)"
        else
            fail "wall-clock exceeded ${max_sec}s (got ${elapsed}s)"
        fi
    fi

    [[ -n "$min_sym" ]] && assert_range "$name symbols" "$min_sym" "$max_sym" "${symbols:-0}"
    [[ -n "$min_edges" ]] && assert_range "$name edges" "$min_edges" "$max_edges" "${edges:-0}"

    # ---- query: well-known symbols ----
    if [[ -n "$wks" && "$wks" != "[]" ]]; then
        head2 "query well-known symbols"
        local clean
        clean=$(echo "$wks" | tr -d '[]"' | tr ',' ' ')
        for sym in $clean; do
            sym=$(echo "$sym" | xargs)
            [[ -z "$sym" ]] && continue
            local q_out q_err
            # Separate stderr — daemon startup banners / progress logs go to
            # stderr and used to pollute the JSON parse. Stdout alone should
            # be either pure JSON (graph result) or the "No symbol found"
            # text path.
            q_err=$(mktemp)
            q_out=$(run_cg "$dir" query "$sym" --output-format json 2>"$q_err") || {
                fail "$name query $sym crashed (stderr: $(head -c 200 "$q_err"))"
                rm -f "$q_err"
                continue
            }
            rm -f "$q_err"
            if echo "$q_out" | grep -q "^No symbol found"; then
                warn "$name: '$sym' not in graph (may be expected if SHA moved)"
            elif echo "$q_out" | jq -e '.center.name' >/dev/null 2>&1; then
                pass "$name: query '$sym' returned a center"
            else
                # Last-chance: surface the first 200 chars for debugging.
                fail "$name: query '$sym' produced unexpected output: $(echo "$q_out" | head -c 200)"
            fi
        done
    fi

    # ---- orphans: just must not crash ----
    head2 "orphans"
    if run_cg "$dir" orphans --public-only --output-format json >/dev/null 2>&1; then
        pass "$name orphans runs clean"
    else
        fail "$name orphans crashed"
    fi

    # ---- snapshot ----
    cat > "$HERE/snapshots/${name}-${sha:0:12}.json" <<EOF
{
  "name": "$name",
  "sha": "$sha",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "symbols": ${symbols:-null},
  "edges": ${edges:-null},
  "wall_clock_sec": $elapsed
}
EOF
    info "snapshot: ${HERE}/snapshots/${name}-${sha:0:12}.json"
}

# ---- main ----
head1 "Tier 2 real-world e2e"
info "binary: $CG_BIN"

total=0
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    total=$((total+1))
    run_project "$line" || true
done < <(parse_projects)

echo
echo "════════════════════════════════════════════"
print_summary

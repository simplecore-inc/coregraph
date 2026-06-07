#!/usr/bin/env bash
# Self-consistency invariants that must hold for any valid graph.
# These run after every scenario regardless of scenario-specific expectations.

# Requires common.sh to be sourced.

# invariant_stats_matches_export <project_dir>
# stats.{symbols,edges} must equal export.{nodes,edges}.length.
invariant_stats_matches_export() {
    local proj="$1"
    # Kill any daemon first: `stats` has an IPC fast-path that reads from a
    # long-lived graph snapshot, while `export` always rebuilds in-process.
    # The two can disagree if a prior command left a daemon that built its
    # graph before the current scenario's fixture was fully in place.
    pkill -9 -f "coregraph server" 2>/dev/null || true
    rm -rf "$proj/.coregraph" 2>/dev/null || true

    local stats_out export_json
    # Use --min-confidence 0.0 on both sides so the default preset threshold
    # (0.85) doesn't filter edges asymmetrically between the two.
    stats_out=$(run_cg "$proj" --min-confidence 0.0 stats --output-format json 2>/dev/null) || {
        fail "invariant: stats failed"
        return
    }
    export_json=$(run_cg "$proj" --min-confidence 0.0 export --format json-graph 2>/dev/null) || {
        fail "invariant: export failed"
        return
    }

    local s_syms s_edges e_nodes e_edges
    s_syms=$(echo "$stats_out" | awk -F': ' '/^symbols:/{print $2}' | tr -d ' ')
    s_edges=$(echo "$stats_out" | awk -F': ' '/^edges:/{print $2}' | tr -d ' ')
    e_nodes=$(echo "$export_json" | jq '.nodes | length' 2>/dev/null || echo "?")
    e_edges=$(echo "$export_json" | jq '.edges | length' 2>/dev/null || echo "?")
    assert_eq "invariant: stats.symbols == export.nodes" "$s_syms" "$e_nodes"
    assert_eq "invariant: stats.edges == export.edges" "$s_edges" "$e_edges"
}

# invariant_no_dangling_edges <project_dir>
# Every edge.source/target must be in nodes set.
invariant_no_dangling_edges() {
    local proj="$1"
    local export_json dangling
    export_json=$(run_cg "$proj" export --format json-graph 2>/dev/null) || {
        fail "invariant: export failed"
        return
    }
    dangling=$(echo "$export_json" | jq '
        (.nodes | map(.id) | unique) as $ids
        | [.edges[] | select((.from | IN($ids[]) | not) or (.to | IN($ids[]) | not))]
        | length
    ' 2>/dev/null || echo "?")
    assert_eq "invariant: no dangling edges" "0" "$dangling"
}

# invariant_query_center_matches <project_dir> <symbol>
invariant_query_center_matches() {
    local proj="$1" sym="$2"
    local out
    out=$(run_cg "$proj" query "$sym" --output-format json 2>/dev/null) || {
        fail "invariant: query failed for $sym"
        return
    }
    local name
    name=$(echo "$out" | jq -r '.center.name' 2>/dev/null)
    assert_eq "invariant: query center.name for $sym" "$sym" "$name"
}

# run_all_invariants <project_dir>
run_all_invariants() {
    local proj="$1"
    head2 "invariants"
    invariant_stats_matches_export "$proj"
    invariant_no_dangling_edges "$proj"
}

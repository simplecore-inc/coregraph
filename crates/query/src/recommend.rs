//! Config-recommendation engine — pure signal computation from the symbol graph.
//!
//! This module is the single source of truth for every signal `coregraph config recommend`
//! surfaces. The four signals are:
//!
//! 1. **Noise candidates** — data-dominated files to add to `[index].exclude`.
//! 2. **String-match cap** — a lower `string_match_max_files` when projected
//!    cross-file string-pair volume exceeds a share threshold.
//! 3. **API-path disable** — disable the api-path inconsistency category when
//!    all near-miss pairs are in the same language family (no cross-language
//!    contract drift detected).
//! 4. **Generated-file candidates** — files whose content contains a standard
//!    generated-file marker to add to `[analysis].exclude`.

use crate::exclude::PathExcluder;
use crate::inconsistencies::{
    find_api_path_mismatches_with, InconsistencyCategory, InconsistencyOptions,
};
use coregraph_core::SymbolKind;
use coregraph_graph::SymbolGraph;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// One data-dominated file suggested for `[index].exclude`.
#[derive(Debug, Clone, Serialize)]
pub struct NoiseFile {
    pub file: PathBuf,
    /// Data-kind symbols in the file (ConfigKey | StringLiteral | DocSection).
    pub data_symbols: u32,
    /// Truncating integer share of ALL graph symbols.
    pub share_pct: u32,
}

/// Projected StringMatch edge volume at one candidate cap.
#[derive(Debug, Clone, Serialize)]
pub struct CapStep {
    pub cap: usize,
    pub projected_edges: u64,
}

/// Recommendation to lower `[index] string_match_max_files`.
#[derive(Debug, Clone, Serialize)]
pub struct CapRecommendation {
    pub current_cap: usize,
    /// Projected cross-file string-pair edges at the current cap.
    pub current_edges: u64,
    pub recommended_cap: usize,
    pub projected_edges: u64,
    pub total_edges: u64,
    /// Projection table for candidate caps (ascending), for transparency.
    pub steps: Vec<CapStep>,
}

/// Recommendation to disable the api-path category. Fires only when every
/// near-miss pair is same-language-family, so `report_count` is also the
/// number of same-family pairs.
#[derive(Debug, Clone, Serialize)]
pub struct ApiPathRecommendation {
    pub report_count: usize,
}

/// A generated file (content marker) suggested for `[analysis].exclude`.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedFile {
    /// Relative to the project root when possible.
    pub file: PathBuf,
    pub marker: String,
    pub symbols: u32,
}

/// Everything `config recommend` derives from one graph.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Recommendations {
    pub index_exclude: Vec<NoiseFile>,
    pub string_match: Option<CapRecommendation>,
    pub disable_api_path: Option<ApiPathRecommendation>,
    pub analysis_exclude: Vec<GeneratedFile>,
}

impl Recommendations {
    pub fn is_empty(&self) -> bool {
        self.index_exclude.is_empty()
            && self.string_match.is_none()
            && self.disable_api_path.is_none()
            && self.analysis_exclude.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Signal 1: Noise candidates (data-dominated files)
// ---------------------------------------------------------------------------

/// A file only counts as a noise candidate when it holds at least this many
/// data-kind symbols — tiny projects are skewed by nature and need no warning.
const NOISE_MIN_SYMBOLS: u32 = 200;
/// ...and its data-kind symbols are at least this share (percent) of ALL
/// symbols in the graph. Kept low: the kind gate below already excludes
/// ordinary code files, so a data bundle worth 5% of the graph is real noise.
/// Compared as a truncating integer percentage (4.9% computes as 4 and does
/// not fire) — deliberately conservative.
const NOISE_MIN_SHARE_PCT: u32 = 5;
/// A file qualifies only when data-kind symbols dominate it (>= this percent
/// of the file's own symbols) — i18n bundles and generated docs are ~100%
/// data kinds, while source files mixing some literals are not flagged.
/// Compared as a truncating integer percentage (89.9% computes as 89 and
/// does not fire) — deliberately conservative.
const NOISE_MIN_DATA_KIND_PCT: u32 = 90;
/// At most this many candidates are reported.
const NOISE_MAX_REPORTED: usize = 5;

/// Files whose data-kind symbols (config keys, string literals, doc
/// sections) contribute an outsized share of the whole graph — typically
/// generated data (i18n bundles, generated docs) that the user may want in
/// `[index].exclude`. Suggestion-only: nothing is excluded automatically.
/// The kind gate means ordinary code files never flag regardless of size,
/// and the thresholds are deliberately conservative.
pub fn noise_candidates(graph: &SymbolGraph) -> Vec<NoiseFile> {
    // Tally and compare in u64: `data * 100` would overflow u32 once a
    // graph holds more than ~42M symbols.
    let total = graph.node_count() as u64;
    if total == 0 {
        return Vec::new();
    }
    // Per file: (all symbols in the file, data-kind symbols in the file).
    let mut per_file: HashMap<PathBuf, (u64, u64)> = HashMap::new();
    for n in graph.nodes() {
        if n.file.as_os_str().is_empty() {
            continue; // synthetic nodes have no file to exclude
        }
        let entry = per_file.entry(n.file.to_path_buf()).or_insert((0, 0));
        entry.0 += 1;
        if matches!(
            n.kind,
            SymbolKind::ConfigKey | SymbolKind::StringLiteral | SymbolKind::DocSection
        ) {
            entry.1 += 1;
        }
    }
    let mut noisy: Vec<NoiseFile> = per_file
        .into_iter()
        .filter(|&(_, (file_total, data))| {
            // `file_total` is structurally >= 1 — the per-file total counter
            // increments before the kind check — so the dominance division
            // below cannot divide by zero.
            data >= NOISE_MIN_SYMBOLS as u64
                && data * 100 / total >= NOISE_MIN_SHARE_PCT as u64
                && data * 100 / file_total >= NOISE_MIN_DATA_KIND_PCT as u64
        })
        .map(|(f, (_, data))| NoiseFile {
            file: f,
            data_symbols: data as u32,
            share_pct: (data * 100 / total) as u32,
        })
        .collect();
    noisy.sort_by_key(|nf| std::cmp::Reverse(nf.data_symbols));
    noisy.truncate(NOISE_MAX_REPORTED);
    noisy
}

// ---------------------------------------------------------------------------
// Signal 2: String-match cap recommendation
// ---------------------------------------------------------------------------

/// Share threshold (percent of all edges) above which the projected
/// string-pair volume at the current cap triggers a recommendation.
const CAP_TRIGGER_SHARE_PCT: u64 = 15;
/// Target share the recommended cap should bring the projection under.
const CAP_TARGET_SHARE_PCT: u64 = 10;
/// Candidate caps evaluated, ascending.
const CAP_CANDIDATES: &[usize] = &[2, 3, 4, 6, 8];

/// Bucket every StringLiteral value to the files of its occurrences (one
/// entry per occurrence, so a value repeated in one file appears repeatedly).
/// Built once per `recommend_string_match_cap` call so projecting several
/// candidate caps does not rescan the whole graph.
fn string_value_buckets(graph: &SymbolGraph) -> HashMap<&str, Vec<&Path>> {
    let mut buckets: HashMap<&str, Vec<&Path>> = HashMap::new();
    for n in graph.nodes() {
        if n.kind == SymbolKind::StringLiteral {
            buckets
                .entry(n.name.as_str())
                .or_default()
                .push(n.file.as_ref());
        }
    }
    buckets
}

/// Projected cross-file string-pair edge count at `cap` (0 = unlimited) from
/// prebuilt value buckets, computed exactly the way
/// `ValueIndex::matching_string_pairs` pairs them: per identical value,
/// all id pairs across different files; the whole value is skipped when
/// it occurs in more than `cap` distinct files.
///
/// NOTE: in a fresh `match_strings` run every i<j id pair is unique, so this
/// projection is exact for the newly inserted StringMatch edges; it can
/// diverge only when the graph still carries StringMatch edges from a prior
/// incremental run.
fn projected_edges_from_buckets(buckets: &HashMap<&str, Vec<&Path>>, cap: usize) -> u64 {
    let mut edges: u64 = 0;
    for files in buckets.values() {
        let n = files.len() as u64;
        if n < 2 {
            continue;
        }
        let distinct: HashSet<&Path> = files.iter().copied().collect();
        if cap > 0 && distinct.len() > cap {
            continue;
        }
        // Cross-file pairs = all pairs minus same-file pairs.
        let mut per_file: HashMap<&Path, u64> = HashMap::new();
        for f in files {
            *per_file.entry(f).or_insert(0) += 1;
        }
        let all = n * (n - 1) / 2;
        let same: u64 = per_file.values().map(|c| c * (c - 1) / 2).sum();
        edges += all - same;
    }
    edges
}

/// Convenience wrapper for a single-cap projection: builds the buckets and
/// projects once. See `projected_edges_from_buckets` for the semantics.
/// Production code goes through `recommend_string_match_cap`, which reuses
/// one bucket build across all candidate caps; only tests project directly.
#[cfg(test)]
fn projected_string_pair_edges(graph: &SymbolGraph, cap: usize) -> u64 {
    projected_edges_from_buckets(&string_value_buckets(graph), cap)
}

/// Recommend a lower `string_match_max_files` when the projection at the
/// current cap exceeds CAP_TRIGGER_SHARE_PCT of all edges: pick the LARGEST
/// candidate cap whose projection is at or under CAP_TARGET_SHARE_PCT
/// (falling back to the smallest candidate). Returns None when the current
/// volume is already acceptable, the graph has no edges, or no candidate
/// cap improves on the current one.
pub fn recommend_string_match_cap(
    graph: &SymbolGraph,
    current_cap: usize,
) -> Option<CapRecommendation> {
    let total_edges = graph.edge_count() as u64;
    if total_edges == 0 {
        return None;
    }
    let buckets = string_value_buckets(graph);
    let current_edges = projected_edges_from_buckets(&buckets, current_cap);
    if current_edges * 100 / total_edges <= CAP_TRIGGER_SHARE_PCT {
        return None;
    }
    let steps: Vec<CapStep> = CAP_CANDIDATES
        .iter()
        .map(|&cap| CapStep {
            cap,
            projected_edges: projected_edges_from_buckets(&buckets, cap),
        })
        .collect();
    let recommended = steps
        .iter()
        .filter(|s| s.cap < current_cap || current_cap == 0)
        .filter(|s| s.projected_edges * 100 / total_edges <= CAP_TARGET_SHARE_PCT)
        .max_by_key(|s| s.cap)
        .or_else(|| steps.first())?
        .clone();
    // A recommendation that cannot improve on the current cap is noise.
    if current_cap != 0 && recommended.cap >= current_cap {
        return None;
    }
    Some(CapRecommendation {
        current_cap,
        current_edges,
        recommended_cap: recommended.cap,
        projected_edges: recommended.projected_edges,
        total_edges,
        steps,
    })
}

// ---------------------------------------------------------------------------
// Signal 3: API-path category disable recommendation
// ---------------------------------------------------------------------------

/// Minimum near-miss report count before the same-family signal is trusted.
const API_PATH_MIN_REPORTS: usize = 3;

/// Language family of a source file, by extension — generic ecosystem
/// grouping, not a framework list. Unknown extensions form their own family
/// (the extension string itself) so two files with the same unknown extension
/// still compare as same-family.
fn language_family(file: &Path) -> String {
    match file.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "js".to_string(),
        "java" | "kt" => "jvm".to_string(),
        "py" | "pyi" => "python".to_string(),
        "go" => "go".to_string(),
        "rs" => "rust".to_string(),
        other => other.to_string(),
    }
}

/// Recommend disabling the api-path category when every near-miss pair is
/// same-language-family. A genuine client/server contract drift usually
/// crosses families (ts <-> java) or at least pairs distinct ecosystems;
/// a report set that never leaves one family is overwhelmingly route-name
/// convention noise (frontend-only repos). Skipped when the category is
/// already disabled in `opts` or there are fewer than API_PATH_MIN_REPORTS.
pub fn recommend_api_path_disable(
    graph: &SymbolGraph,
    opts: &InconsistencyOptions,
) -> Option<ApiPathRecommendation> {
    if opts.disabled.contains(&InconsistencyCategory::ApiPath) {
        return None;
    }
    let reports = find_api_path_mismatches_with(graph, opts);
    if reports.len() < API_PATH_MIN_REPORTS {
        return None;
    }
    let all_same_family = reports.iter().all(|r| {
        language_family(r.node_a.file.as_ref()) == language_family(r.node_b.file.as_ref())
    });
    if !all_same_family {
        return None;
    }
    Some(ApiPathRecommendation {
        report_count: reports.len(),
    })
}

// ---------------------------------------------------------------------------
// Signal 4: Generated-file candidates
// ---------------------------------------------------------------------------

/// Ecosystem-standard generated-file content markers (checked in the first
/// HEAD_BYTES of each indexed file — content-based, never path-based).
const GENERATED_MARKERS: &[&str] = &["@generated", "DO NOT EDIT", "Code generated by"];
const GENERATED_HEAD_BYTES: usize = 2048;
const GENERATED_MAX_REPORTED: usize = 10;

/// Indexed files carrying a generated-content marker AND at least one
/// non-data symbol — candidates for `[analysis].exclude` (still parsed, so
/// their imports keep hand-written symbols connected, but their own symbols
/// stay out of dead-code reports). Files already matched by the project's
/// analysis excluder are skipped. Reads at most GENERATED_HEAD_BYTES per
/// distinct file.
pub fn generated_file_candidates(graph: &SymbolGraph, root: &Path) -> Vec<GeneratedFile> {
    // Graph node paths are assumed absolute — the production pipeline
    // guarantees it (collect_sources walks the canonical project root).
    // A relative-path node would silently bypass the analysis excluder
    // below and resolve the head read against the process cwd instead.
    // Collect per-file: (total symbol count, has at least one non-data symbol).
    let mut per_file: HashMap<PathBuf, (u32, bool)> = HashMap::new();
    for n in graph.nodes() {
        if n.file.as_os_str().is_empty() {
            continue; // skip synthetic nodes
        }
        let entry = per_file.entry(n.file.to_path_buf()).or_insert((0, false));
        entry.0 += 1;
        let is_data = matches!(
            n.kind,
            SymbolKind::ConfigKey | SymbolKind::StringLiteral | SymbolKind::DocSection
        );
        if !is_data {
            entry.1 = true;
        }
    }

    let analysis_excluder = PathExcluder::analysis_from_project_root(root);

    let mut results: Vec<GeneratedFile> = Vec::new();
    for (path, (symbol_count, has_non_data)) in per_file {
        if !has_non_data {
            // Data-only files are not candidates: generated data files that
            // only contain config keys / string literals are typically already
            // handled by the noise-candidates signal (signal 1).
            continue;
        }
        if analysis_excluder.is_excluded(&path) {
            continue;
        }
        // Read the file head and look for a generated-content marker.
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut head_bytes = Vec::with_capacity(GENERATED_HEAD_BYTES);
        let _ = file
            .take(GENERATED_HEAD_BYTES as u64)
            .read_to_end(&mut head_bytes);
        let head = String::from_utf8_lossy(&head_bytes);
        let Some(marker) = GENERATED_MARKERS.iter().find(|&&m| head.contains(m)) else {
            continue;
        };
        let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        results.push(GeneratedFile {
            file: relative,
            marker: marker.to_string(),
            symbols: symbol_count,
        });
    }
    results.sort_by_key(|gf| std::cmp::Reverse(gf.symbols));
    results.truncate(GENERATED_MAX_REPORTED);
    results
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Compute every recommendation from one graph. `current_cap` is the
/// project's effective `[index] string_match_max_files`; `opts` the
/// project's inconsistency options. Pure suggestion — nothing is written.
pub fn recommend(
    graph: &SymbolGraph,
    root: &Path,
    current_cap: usize,
    opts: &InconsistencyOptions,
) -> Recommendations {
    Recommendations {
        index_exclude: noise_candidates(graph),
        string_match: recommend_string_match_cap(graph, current_cap),
        disable_api_path: recommend_api_path_disable(graph, opts),
        analysis_exclude: generated_file_candidates(graph, root),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolNode};
    use std::io::Write;
    use std::path::PathBuf;

    // Helper: build a graph with nodes from a slice of (file, kind, count).
    fn graph_with(counts: &[(&str, SymbolKind, u32)]) -> SymbolGraph {
        let mut g = SymbolGraph::new();
        for (file, kind, n) in counts {
            for i in 0..*n {
                g.insert_node(SymbolNode::new(
                    SymbolId(0),
                    kind.clone(),
                    format!("{file}-{i}"),
                    *file,
                    i,
                    i + 1,
                ));
            }
        }
        g
    }

    // ---------------------------------------------------------------------------
    // Signal 1: noise_candidates (moved from index.rs)
    // ---------------------------------------------------------------------------

    #[test]
    fn noise_candidates_flags_data_dominated_files_only() {
        // A 500-key i18n bundle in a 1000-symbol graph flags (50% share,
        // 100% data kinds); plain code files never flag regardless of size.
        let g = graph_with(&[
            ("locales/ko.json", SymbolKind::ConfigKey, 500),
            ("src/a.ts", SymbolKind::Function, 300),
            ("src/b.ts", SymbolKind::Function, 200),
        ]);
        let noisy = noise_candidates(&g);
        assert_eq!(noisy.len(), 1, "{noisy:?}");
        assert!(noisy[0].file.ends_with("locales/ko.json"));
        assert_eq!(noisy[0].share_pct, 50, "share percentage");
    }

    #[test]
    fn noise_candidates_catches_moderate_share_data_bundles() {
        // The field case that motivated the kind gate: a generated bundle
        // worth ~7% of all symbols must flag once it is data-dominated and
        // above the absolute floor.
        let code_files: Vec<String> = (0..9).map(|i| format!("src/f{i}.ts")).collect();
        let mut entries: Vec<(&str, SymbolKind, u32)> =
            vec![("locales/en.json", SymbolKind::ConfigKey, 210)];
        for f in &code_files {
            entries.push((f.as_str(), SymbolKind::Function, 300));
        }
        let g = graph_with(&entries);
        let noisy = noise_candidates(&g);
        assert_eq!(noisy.len(), 1, "{noisy:?}");
        assert!(noisy[0].file.ends_with("locales/en.json"));
        assert_eq!(noisy[0].share_pct, 7, "share percentage");
    }

    #[test]
    fn noise_candidates_quiet_on_small_code_or_mixed_graphs() {
        // Small absolute counts never trigger.
        let small = graph_with(&[
            ("a.json", SymbolKind::ConfigKey, 90),
            ("b.ts", SymbolKind::Function, 10),
        ]);
        assert!(noise_candidates(&small).is_empty());
        // Large balanced CODE graphs never trigger (kind gate).
        let code = graph_with(&[
            ("a.ts", SymbolKind::Function, 300),
            ("b.ts", SymbolKind::Function, 300),
            ("c.ts", SymbolKind::Class, 300),
            ("d.ts", SymbolKind::Method, 300),
        ]);
        assert!(noise_candidates(&code).is_empty());
        // A file mixing literals into code stays quiet via the kind-
        // dominance gate even when its data symbols clear the other floors.
        let mixed = graph_with(&[
            ("src/big.ts", SymbolKind::StringLiteral, 250),
            ("src/big.ts", SymbolKind::Function, 100),
        ]);
        assert!(noise_candidates(&mixed).is_empty());
    }

    #[test]
    fn noise_candidates_respects_min_share_floor() {
        // A 200-key bundle under the graph-share floor (200/8000 = 2%)
        // stays quiet even though it clears the absolute floor and the
        // kind-dominance gate.
        let code_files: Vec<String> = (0..26).map(|i| format!("src/f{i}.ts")).collect();
        let mut entries: Vec<(&str, SymbolKind, u32)> =
            vec![("locales/ko.json", SymbolKind::ConfigKey, 200)];
        for f in &code_files {
            entries.push((f.as_str(), SymbolKind::Function, 300));
        }
        let g = graph_with(&entries);
        assert!(noise_candidates(&g).is_empty());
    }

    // ---------------------------------------------------------------------------
    // Signal 2: recommend_string_match_cap
    // ---------------------------------------------------------------------------

    /// Add a real edge between two nodes so `graph.edge_count()` counts it.
    fn add_dummy_edge(g: &mut SymbolGraph, from: SymbolId, to: SymbolId) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.85),
            PathBuf::from("dummy.rs"),
        ));
    }

    #[test]
    fn signal2_fires_above_trigger() {
        // One string value "hello" in 6 distinct files → 15 projected
        // cross-file pairs. 80 real code edges put the share at
        // 15*100/80 = 18% > the 15% trigger, so a lower cap is recommended.
        let mut g = SymbolGraph::new();
        for i in 0..6u32 {
            g.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::StringLiteral,
                "hello",
                format!("f{i}.ts"),
                0,
                5,
            ));
        }
        // Function nodes to carry the real edges, distinct from the literals.
        let mut code_ids: Vec<SymbolId> = Vec::new();
        for i in 0..20u32 {
            let id = g.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                format!("fn_{i}"),
                format!("code{i}.ts"),
                0,
                10,
            ));
            code_ids.push(id);
        }
        // Distinct (from, to) combinations — insert_edge dedups repeats.
        let mut added = 0;
        'outer: for i in 0..code_ids.len() {
            for j in 0..code_ids.len() {
                if i != j {
                    add_dummy_edge(&mut g, code_ids[i], code_ids[j]);
                    added += 1;
                    if added >= 80 {
                        break 'outer;
                    }
                }
            }
        }
        assert_eq!(g.edge_count(), 80, "edge count should be 80");
        let rec = recommend_string_match_cap(&g, 10);
        assert!(rec.is_some(), "expected a recommendation");
        let r = rec.unwrap();
        assert_eq!(r.current_edges, 15);
        assert_eq!(r.total_edges, 80);
        assert!(r.recommended_cap < 10, "recommended cap should be lower");
    }

    #[test]
    fn signal2_none_below_trigger() {
        // One string value in 2 files = 1 projected pair.
        // Add many distinct real edges so share = 1/N < 15% → no recommendation.
        let mut g = SymbolGraph::new();
        let _id_a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "world",
            "a.ts",
            0,
            5,
        ));
        let _id_b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "world",
            "b.ts",
            0,
            5,
        ));
        // Insert 20 Function nodes to generate 20*(20-1) = 380 distinct edges.
        let mut code_ids: Vec<SymbolId> = Vec::new();
        for i in 0..20u32 {
            let id = g.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                format!("fn_{i}"),
                format!("code{i}.ts"),
                0,
                10,
            ));
            code_ids.push(id);
        }
        for i in 0..code_ids.len() {
            for j in 0..code_ids.len() {
                if i != j {
                    add_dummy_edge(&mut g, code_ids[i], code_ids[j]);
                }
            }
        }
        // projected = 1, total_edges >= 100; share < 15% → None
        let total = g.edge_count() as u64;
        assert!(total >= 100, "need enough edges so share < 15%");
        assert!(recommend_string_match_cap(&g, 8).is_none());
    }

    #[test]
    fn signal2_none_when_current_cap_cannot_be_lowered() {
        // 30 string values, each in exactly 2 distinct files, survive even
        // the smallest candidate cap (2) — 30 projected pairs. With 100 real
        // edges the share is 30% > trigger, but no candidate cap is below
        // current_cap=2; recommending cap 2 again would be a no-op, so the
        // function must return None.
        let mut g = SymbolGraph::new();
        for v in 0..30u32 {
            for f in 0..2u32 {
                g.insert_node(SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::StringLiteral,
                    format!("value_{v}"),
                    format!("pair{v}_{f}.ts"),
                    0,
                    5,
                ));
            }
        }
        let mut code_ids: Vec<SymbolId> = Vec::new();
        for i in 0..15u32 {
            let id = g.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                format!("fn_{i}"),
                format!("code{i}.ts"),
                0,
                10,
            ));
            code_ids.push(id);
        }
        let mut added = 0;
        'outer: for i in 0..code_ids.len() {
            for j in 0..code_ids.len() {
                if i != j {
                    add_dummy_edge(&mut g, code_ids[i], code_ids[j]);
                    added += 1;
                    if added >= 100 {
                        break 'outer;
                    }
                }
            }
        }
        assert_eq!(g.edge_count(), 100, "edge count should be 100");
        // Trigger precondition: 30 projected pairs / 100 edges = 30% > 15%.
        assert_eq!(projected_string_pair_edges(&g, 2), 30);
        assert!(recommend_string_match_cap(&g, 2).is_none());
    }

    #[test]
    fn signal2_cross_file_pair_math() {
        // 2 files × 2 occurrences each = 4 node ids total.
        // all pairs = 4*3/2 = 6; same-file pairs = 2*(2-1)/2 * 2 files = 2;
        // cross-file pairs = 6 - 2 = 4.
        let mut g = SymbolGraph::new();
        let mut ids = Vec::new();
        for i in 0..2u32 {
            for _ in 0..2u32 {
                let id = g.insert_node(SymbolNode::new(
                    SymbolId(0),
                    SymbolKind::StringLiteral,
                    "shared",
                    format!("file{i}.ts"),
                    0,
                    6,
                ));
                ids.push(id);
            }
        }
        // Add some real edges for total_edges > 0 (not needed for the math
        // assertion but good practice).
        add_dummy_edge(&mut g, ids[0], ids[1]);
        let cross = projected_string_pair_edges(&g, 0);
        assert_eq!(cross, 4, "expected 4 cross-file pairs");
    }

    // ---------------------------------------------------------------------------
    // Signal 3: recommend_api_path_disable
    // ---------------------------------------------------------------------------

    fn insert_api_path(g: &mut SymbolGraph, raw_path: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            format!("api_path::{}", raw_path),
            PathBuf::from(file),
            0,
            raw_path.len() as u32,
        ))
    }

    fn default_opts() -> InconsistencyOptions {
        InconsistencyOptions {
            disabled: vec![],
            api_path_min_segments: 2,
        }
    }

    #[test]
    fn signal3_fires_on_three_same_family_pairs() {
        // Three near-miss pairs all in .ts / .tsx (same js family) → Some.
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "client.ts");
        insert_api_path(&mut g, "/api/v1/card", "server.tsx");
        insert_api_path(&mut g, "/api/v1/users", "client2.ts");
        insert_api_path(&mut g, "/api/v1/user", "server2.tsx");
        insert_api_path(&mut g, "/api/v1/orders", "client3.ts");
        insert_api_path(&mut g, "/api/v1/order", "server3.tsx");
        let opts = default_opts();
        let rec = recommend_api_path_disable(&g, &opts);
        assert!(
            rec.is_some(),
            "expected recommendation for same-family pairs"
        );
        let r = rec.unwrap();
        assert_eq!(r.report_count, 3);
    }

    #[test]
    fn signal3_none_below_floor() {
        // Only 2 same-family pairs — below API_PATH_MIN_REPORTS (3).
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "a.ts");
        insert_api_path(&mut g, "/api/v1/card", "b.ts");
        insert_api_path(&mut g, "/api/v1/users", "c.ts");
        insert_api_path(&mut g, "/api/v1/user", "d.ts");
        // These happen to produce 2 near-miss pairs (cards/card, users/user).
        let opts = default_opts();
        let rec = recommend_api_path_disable(&g, &opts);
        assert!(rec.is_none(), "expected None for < 3 pairs");
    }

    #[test]
    fn signal3_none_on_mixed_family() {
        // 3+ near-miss pairs but one crosses families (ts <-> java) → None.
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "client.ts");
        insert_api_path(&mut g, "/api/v1/card", "server.tsx");
        insert_api_path(&mut g, "/api/v1/users", "client2.ts");
        insert_api_path(&mut g, "/api/v1/user", "server2.tsx");
        insert_api_path(&mut g, "/api/v1/orders", "service.java");
        insert_api_path(&mut g, "/api/v1/order", "client3.ts");
        let opts = default_opts();
        let rec = recommend_api_path_disable(&g, &opts);
        assert!(rec.is_none(), "expected None when families differ");
    }

    #[test]
    fn signal3_none_when_already_disabled() {
        let mut g = SymbolGraph::new();
        insert_api_path(&mut g, "/api/v1/cards", "a.ts");
        insert_api_path(&mut g, "/api/v1/card", "b.ts");
        insert_api_path(&mut g, "/api/v1/users", "c.ts");
        insert_api_path(&mut g, "/api/v1/user", "d.ts");
        insert_api_path(&mut g, "/api/v1/orders", "e.ts");
        insert_api_path(&mut g, "/api/v1/order", "f.ts");
        let opts = InconsistencyOptions {
            disabled: vec![InconsistencyCategory::ApiPath],
            api_path_min_segments: 2,
        };
        assert!(recommend_api_path_disable(&g, &opts).is_none());
    }

    // ---------------------------------------------------------------------------
    // Signal 4: generated_file_candidates + recommend() smoke test
    // ---------------------------------------------------------------------------

    #[test]
    fn signal4_flags_generated_file_with_code_symbol() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Write a file that has a generated marker AND a code symbol.
        let gen_path = root.join("src").join("routeTree.gen.ts");
        std::fs::create_dir_all(gen_path.parent().unwrap()).unwrap();
        {
            let mut f = std::fs::File::create(&gen_path).unwrap();
            writeln!(f, "// Code generated by tanstack. DO NOT EDIT.").unwrap();
            writeln!(f, "export function getRoutes() {{}}").unwrap();
        }

        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "getRoutes",
            gen_path.clone(),
            0,
            10,
        ));

        let results = generated_file_candidates(&g, root);
        assert_eq!(
            results.len(),
            1,
            "expected the generated file to be flagged"
        );
        // relative path should be src/routeTree.gen.ts
        assert_eq!(results[0].file, PathBuf::from("src/routeTree.gen.ts"));
        // marker must be one of the recognised generated markers
        assert!(
            GENERATED_MARKERS.contains(&results[0].marker.as_str()),
            "unexpected marker: {}",
            results[0].marker
        );
    }

    #[test]
    fn signal4_data_only_not_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Write a file with a generated marker but only data symbols (ConfigKey).
        let data_path = root.join("config.generated.json");
        {
            let mut f = std::fs::File::create(&data_path).unwrap();
            writeln!(f, "// @generated").unwrap();
            writeln!(f, "{{\"key\": \"value\"}}").unwrap();
        }

        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            "key",
            data_path.clone(),
            0,
            5,
        ));

        let results = generated_file_candidates(&g, root);
        assert!(
            results.is_empty(),
            "data-only marker file should not be flagged"
        );
    }

    #[test]
    fn signal4_excluded_file_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // Write a coregraph config that analysis-excludes the generated file.
        std::fs::create_dir_all(root.join(".coregraph")).unwrap();
        std::fs::write(
            root.join(".coregraph").join("config.toml"),
            "[analysis]\nexclude = [\"src/gen.ts\"]\n",
        )
        .unwrap();

        let gen_path = root.join("src").join("gen.ts");
        std::fs::create_dir_all(gen_path.parent().unwrap()).unwrap();
        {
            let mut f = std::fs::File::create(&gen_path).unwrap();
            writeln!(f, "// DO NOT EDIT").unwrap();
            writeln!(f, "export function foo() {{}}").unwrap();
        }

        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            gen_path.clone(),
            0,
            10,
        ));

        let results = generated_file_candidates(&g, root);
        assert!(results.is_empty(), "excluded file should be skipped");
    }

    #[test]
    fn recommend_smoke_empty_graph_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = SymbolGraph::new();
        let opts = InconsistencyOptions::default();
        let recs = recommend(&g, root, 8, &opts);
        assert!(
            recs.is_empty(),
            "empty graph should produce no recommendations"
        );
    }
}

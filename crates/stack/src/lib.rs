//! Cross-file name resolution for CoreGraph.
//!
//! Two backends live here:
//! - [`resolver`] — the identifier-matching syntactic fallback.
//! - [`backend`] — the pluggable `ResolutionBackend` trait plus a
//!   stack-graphs-backed implementation. Java / TS / JS / Python use the
//!   upstream `tree-sitter-stack-graphs` rules; Go / Rust / Kotlin use
//!   CoreGraph's own hand-authored `.tsg` rules (no upstream package
//!   exists). All of these route through [`backend::StackGraphsBackend`].
//!
//! The extractor pipeline picks a backend via [`backend::StackGraphsBackend`]
//! for those supported languages and falls back to
//! [`backend::SyntacticBackend`] for anything else there are no
//! stack-graphs rules for. Each resolved ref carries its own origin
//! (`NameResolved` for stitched refs, `SyntaxMatched` for fallback refs),
//! so a mixed batch keeps fallback edges honest rather than relying on a
//! single global success flag.

pub mod backend;
pub mod resolver;
pub use backend::{
    rust_module_globals, BuildReport, ResolutionBackend, StackGraphsBackend, SyntacticBackend,
};
pub use resolver::{resolve_files, resolve_files_with_graph, ResolutionResult, ResolvedRef};

use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind};
use coregraph_graph::{EdgeEvaluator, SymbolGraph};

/// Report returned by `apply_resolutions` — what actually made it
/// into the graph and where matching dropped entries. Exposed so
/// callers (and tests) can measure the round-trip hit rate.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// Number of `Resolves` edges actually inserted.
    pub edges_added: usize,
    /// Refs whose `from_span` matched a SymbolNode.
    pub from_hits: usize,
    /// Refs whose `to_span` matched a SymbolNode.
    pub to_hits: usize,
    /// Refs that matched both endpoints.
    pub both_hits: usize,
    /// Total refs fed in.
    pub total_refs: usize,
}

/// Apply resolved cross-file references as `Resolves` edges on the
/// `SymbolGraph`. Matching rules:
///
/// - `from_span == (0, 0)`: legacy whole-file marker. The endpoint is
///   the file/module-level node if one exists, otherwise the outermost
///   enclosing node in the file.
/// - Otherwise: pick the **most-specific** (smallest-span) node whose
///   span matches via [`endpoint_matches`]. Stack-graphs emits identifier
///   token spans, which sit inside the defining node's recorded span; the
///   old behaviour matched *every* ancestor (file → class → method) and
///   emitted a Cartesian product of edges, exploding into 100M+ edges on
///   real projects. One-ref-one-edge avoids that.
///
/// Returns an `ApplyReport` so callers can measure precision.
pub fn apply_resolutions_report(graph: &mut SymbolGraph, result: &ResolutionResult) -> ApplyReport {
    let mut report = ApplyReport {
        total_refs: result.refs.len(),
        ..Default::default()
    };

    // Pre-build a per-file node list from the evidence index so each
    // select_endpoint call is O(nodes_in_file) rather than O(total_nodes).
    // This brings apply_resolutions from O(refs × total_nodes) to
    // O(total_nodes + refs × nodes_in_file) — a ~N-fold speedup when the
    // average file is 1/N of all nodes (here N ≈ 195 files).
    let file_nodes: std::collections::HashMap<std::path::PathBuf, Vec<coregraph_core::SymbolNode>> = {
        // Collect every unique file path referenced by the result's refs.
        let mut nodes_by_file: std::collections::HashMap<
            std::path::PathBuf,
            Vec<coregraph_core::SymbolNode>,
        > = std::collections::HashMap::new();
        // Use the evidence index to look up defined nodes per file in O(1).
        for r in &result.refs {
            for file in [&r.from_file, &r.to_file] {
                if nodes_by_file.contains_key(file.as_path()) {
                    continue;
                }
                let ns: Vec<coregraph_core::SymbolNode> =
                    match graph.evidence_index().evidence_for(file) {
                        Some(ev) => ev
                            .defined_nodes
                            .iter()
                            .filter_map(|id| graph.get_node(*id).cloned())
                            .collect(),
                        // File not in the graph at all — leave empty so
                        // select_endpoint_indexed returns None quickly.
                        None => Vec::new(),
                    };
                nodes_by_file.insert(file.clone(), ns);
            }
        }
        nodes_by_file
    };

    for r in &result.refs {
        let empty = Vec::new();
        let from_nodes = file_nodes.get(r.from_file.as_path()).unwrap_or(&empty);
        let to_nodes = file_nodes.get(r.to_file.as_path()).unwrap_or(&empty);

        let from_id = select_endpoint_indexed(from_nodes, r.from_span);
        let to_id = select_endpoint_indexed(to_nodes, r.to_span);

        if from_id.is_some() {
            report.from_hits += 1;
        }
        if to_id.is_some() {
            report.to_hits += 1;
        }
        let (Some(from), Some(to)) = (from_id, to_id) else {
            continue;
        };
        report.both_hits += 1;
        if from == to {
            continue;
        }

        // Per-ref origin: stack-graphs-stitched refs are NameResolved (0.95),
        // syntactic-fallback refs are SyntaxMatched (0.85). Using the ref's own
        // origin (rather than a single global `success` flag) keeps a mixed
        // result honest — fallback edges for languages without stack-graphs
        // rules stay at 0.85 even when another language stitched successfully.
        let origin = r.origin;
        let confidence = EdgeEvaluator::evaluate(EdgeKind::Resolves, origin);
        let edge = DirectEdge::new(
            from,
            to,
            EdgeKind::Resolves,
            origin,
            confidence,
            r.from_file.clone(),
        );
        if graph.insert_edge(edge) {
            report.edges_added += 1;
        }
    }
    report
}

/// Pick the single SymbolNode from a pre-filtered per-file node slice that an
/// external reference should attach to. Identical selection policy to
/// `select_endpoint` but avoids the O(total_nodes) graph scan.
fn select_endpoint_indexed(
    candidates_unfiltered: &[coregraph_core::SymbolNode],
    span: (u32, u32),
) -> Option<SymbolId> {
    let candidates: Vec<&coregraph_core::SymbolNode> = candidates_unfiltered
        .iter()
        .filter(|n| endpoint_matches(n.span_start, n.span_end, span))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    if span == (0, 0) {
        // Whole-file marker: prefer an explicit File/Module node.
        if let Some(top) = candidates
            .iter()
            .find(|n| matches!(n.kind, SymbolKind::File | SymbolKind::Module))
        {
            return Some(top.id);
        }
        // Otherwise take the outermost (largest-span) node so this edge
        // lands on a container rather than an arbitrary child symbol.
        return candidates
            .iter()
            .max_by_key(|n| n.span_end.saturating_sub(n.span_start))
            .map(|n| n.id);
    }
    // Normal case: smallest enclosing span is the most-specific match.
    candidates
        .iter()
        .min_by_key(|n| n.span_end.saturating_sub(n.span_start))
        .map(|n| n.id)
}

/// Legacy wrapper — returns just the edge count.
pub fn apply_resolutions(graph: &mut SymbolGraph, result: &ResolutionResult) -> usize {
    apply_resolutions_report(graph, result).edges_added
}

/// Endpoint matching policy. `(0, 0)` means "whole file"; otherwise
/// the reference span must sit inside the node's span, OR the node
/// span must sit inside the reference span (extractors sometimes
/// record the identifier, sometimes the declaration block — this
/// handles both directions).
fn endpoint_matches(node_start: u32, node_end: u32, ref_span: (u32, u32)) -> bool {
    if ref_span == (0, 0) {
        return true;
    }
    let (rs, re) = ref_span;
    let node_contains_ref = node_start <= rs && re <= node_end.max(node_start);
    let ref_contains_node = rs <= node_start && node_end <= re.max(rs);
    node_contains_ref || ref_contains_node
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::AnalysisOrigin;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn make_graph() -> SymbolGraph {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserController",
            "A.java",
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserService",
            "B.java",
            0,
            10,
        ));
        g
    }

    #[test]
    fn apply_resolutions_adds_edges() {
        let mut graph = make_graph();
        let result = ResolutionResult {
            refs: vec![ResolvedRef {
                from_file: PathBuf::from("A.java"),
                from_span: (0, 0),
                to_file: PathBuf::from("B.java"),
                to_span: (0, 0),
                origin: AnalysisOrigin::NameResolved,
            }],
            success: true,
        };
        let count = apply_resolutions(&mut graph, &result);
        assert!(count > 0, "should add at least 1 Resolves edge");
    }

    #[test]
    fn apply_resolutions_empty_result_no_edges() {
        let mut graph = make_graph();
        let result = ResolutionResult {
            refs: vec![],
            success: true,
        };
        let count = apply_resolutions(&mut graph, &result);
        assert_eq!(count, 0);
    }

    #[test]
    fn endpoint_matches_whole_file_sentinel() {
        // (0, 0) means "match any node in this file" — preserved for
        // syntactic fallback which only knows the file, not the span.
        assert!(endpoint_matches(10, 20, (0, 0)));
    }

    #[test]
    fn endpoint_matches_node_enclosing_ref_span() {
        // stack-graphs emits identifier spans that sit inside the
        // extractor's recorded node span.
        assert!(endpoint_matches(100, 200, (120, 130)));
    }

    #[test]
    fn endpoint_matches_ref_span_enclosing_node() {
        // Some extractors record just the identifier token, and
        // stack-graphs reports a larger decl range. Accept both
        // directions — this is the "node inside ref" case.
        assert!(endpoint_matches(120, 125, (100, 200)));
    }

    #[test]
    fn endpoint_matches_rejects_disjoint_spans() {
        assert!(!endpoint_matches(100, 150, (200, 250)));
        assert!(!endpoint_matches(200, 250, (100, 150)));
    }

    #[test]
    fn apply_resolutions_report_counts_precision() {
        // Two refs: one whose from/to land on known nodes (both hit),
        // one that points to a file without a matching node (neither).
        let mut graph = make_graph();
        let result = ResolutionResult {
            refs: vec![
                // Hit — A.java → B.java, spans enclose existing nodes
                // (span 0..10 matches both make_graph() entries).
                ResolvedRef {
                    from_file: PathBuf::from("A.java"),
                    from_span: (0, 5),
                    to_file: PathBuf::from("B.java"),
                    to_span: (0, 5),
                    origin: AnalysisOrigin::NameResolved,
                },
                // Miss — unknown file.
                ResolvedRef {
                    from_file: PathBuf::from("A.java"),
                    from_span: (0, 5),
                    to_file: PathBuf::from("C.java"),
                    to_span: (0, 5),
                    origin: AnalysisOrigin::NameResolved,
                },
            ],
            success: true,
        };
        let report = apply_resolutions_report(&mut graph, &result);
        assert_eq!(report.total_refs, 2);
        assert_eq!(report.from_hits, 2, "both refs' from-side hit A.java");
        assert_eq!(report.to_hits, 1, "only the A→B ref hit on the to-side");
        assert_eq!(report.both_hits, 1);
        assert_eq!(report.edges_added, 1);
    }

    #[test]
    fn apply_resolutions_labels_origin_per_ref() {
        // A mixed batch: one stack-graphs-stitched ref (NameResolved 0.95) and
        // one syntactic-fallback ref (SyntaxMatched 0.85). Each edge must carry
        // its OWN origin — the syntactic edge must NOT be promoted to 0.95 just
        // because the batch also contains a stitched ref. This guards the
        // per-ref labeling that replaced the old global `success` flag.
        let mut g = SymbolGraph::new();
        let _a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "fa",
            "a.go",
            0,
            10,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "fb",
            "b.go",
            0,
            10,
        ));
        let c = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "fc",
            "c.go",
            0,
            10,
        ));
        let result = ResolutionResult {
            refs: vec![
                ResolvedRef {
                    from_file: PathBuf::from("a.go"),
                    from_span: (0, 5),
                    to_file: PathBuf::from("b.go"),
                    to_span: (0, 5),
                    origin: AnalysisOrigin::NameResolved,
                },
                ResolvedRef {
                    from_file: PathBuf::from("a.go"),
                    from_span: (0, 5),
                    to_file: PathBuf::from("c.go"),
                    to_span: (0, 5),
                    origin: AnalysisOrigin::SyntaxMatched,
                },
            ],
            // Global flag true (a stitch happened) — must NOT leak into the
            // syntactic ref's label.
            success: true,
        };
        apply_resolutions(&mut g, &result);

        let to_b = g.edges().find(|e| e.to == b).expect("a.go → b.go edge");
        let to_c = g.edges().find(|e| e.to == c).expect("a.go → c.go edge");
        assert_eq!(
            to_b.origin,
            AnalysisOrigin::NameResolved,
            "stitched ref keeps 0.95"
        );
        assert_eq!(
            to_c.origin,
            AnalysisOrigin::SyntaxMatched,
            "fallback ref stays 0.85 even though global success=true"
        );
    }
}

use crate::symbol_graph::SymbolGraph;
use coregraph_core::SymbolId;

/// Quantifies the potential blast radius when a symbol changes.
/// Higher score = more risky to modify.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskScore {
    /// Final composite score (unbounded above, >= 0.0)
    pub score: f32,
    /// Number of edges involving this node
    pub edge_count: usize,
    /// Fraction of edges that cross file boundaries (0.0–1.0)
    pub cross_file_fraction: f32,
    /// Average confidence across all edges involving this node
    pub confidence_avg: f32,
}

impl RiskScore {
    pub fn zero() -> Self {
        Self {
            score: 0.0,
            edge_count: 0,
            cross_file_fraction: 0.0,
            confidence_avg: 0.0,
        }
    }
}

/// Compute the RiskScore for `node_id` in `graph`.
/// Returns RiskScore::zero() if the node has no edges or does not exist.
pub fn score_node(graph: &SymbolGraph, node_id: SymbolId) -> RiskScore {
    let node = match graph.get_node(node_id) {
        Some(n) => n,
        None => return RiskScore::zero(),
    };

    let involved_edges: Vec<_> = graph
        .edges()
        .filter(|e| e.from == node_id || e.to == node_id)
        .collect();

    let edge_count = involved_edges.len();
    if edge_count == 0 {
        return RiskScore::zero();
    }

    let confidence_sum: f32 = involved_edges.iter().map(|e| e.confidence.0).sum();
    let confidence_avg = confidence_sum / edge_count as f32;

    let cross_file_count = involved_edges
        .iter()
        .filter(|e| {
            let other_id = if e.from == node_id { e.to } else { e.from };
            graph
                .get_node(other_id)
                .map(|other| other.file != node.file)
                .unwrap_or(false)
        })
        .count();

    let cross_file_fraction = cross_file_count as f32 / edge_count as f32;
    let score = confidence_avg * (1.0 + edge_count as f32) * (1.0 + cross_file_fraction);

    RiskScore {
        score,
        edge_count,
        cross_file_fraction,
        confidence_avg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_node(g: &mut SymbolGraph, name: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from(file),
            0,
            10,
        ))
    }

    fn insert_edge(g: &mut SymbolGraph, from: SymbolId, to: SymbolId, conf: f32) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(conf),
            PathBuf::from("src/a.rs"),
        ));
    }

    #[test]
    fn orphan_node_zero_risk() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a", "src/a.rs");
        let score = score_node(&g, a);
        assert_eq!(score.score, 0.0);
        assert_eq!(score.edge_count, 0);
    }

    #[test]
    fn single_edge_score() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a", "src/a.rs");
        let b = insert_node(&mut g, "b", "src/a.rs"); // same file
        insert_edge(&mut g, a, b, 1.0);
        let score = score_node(&g, a);
        // confidence_avg=1.0, edge_count=1, cross_file=0.0
        // score = 1.0 * (1+1) * (1+0) = 2.0
        assert!((score.score - 2.0).abs() < 0.01);
        assert_eq!(score.edge_count, 1);
        assert_eq!(score.cross_file_fraction, 0.0);
    }

    #[test]
    fn cross_file_increases_score() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a", "src/a.rs");
        let b = insert_node(&mut g, "b", "src/b.rs"); // different file
        insert_edge(&mut g, a, b, 1.0);
        let score = score_node(&g, a);
        // confidence_avg=1.0, edge_count=1, cross_file=1.0
        // score = 1.0 * (1+1) * (1+1) = 4.0
        assert!((score.score - 4.0).abs() < 0.01);
        assert_eq!(score.cross_file_fraction, 1.0);
    }

    #[test]
    fn unknown_node_returns_zero() {
        let g = SymbolGraph::new();
        let score = score_node(&g, SymbolId(999));
        assert_eq!(score.score, 0.0);
    }
}

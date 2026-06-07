use crate::edge_evaluator::EdgeEvaluator;
use crate::symbol_graph::SymbolGraph;
use coregraph_core::edge::AnalysisOrigin;
use coregraph_core::{DirectEdge, EdgeKind, SymbolId};
use std::path::PathBuf;

pub mod docker_compose;
pub mod go_di;
pub mod react_router;
pub mod spring_config;
pub mod spring_di;

/// A detected mediator pattern connection.
pub struct MediatorEdge {
    pub from: SymbolId,
    pub to: SymbolId,
    pub evidence_file: PathBuf,
}

/// Detects structural patterns in the SymbolGraph and returns Configures edges.
pub trait Mediator {
    /// Scan the graph and return detected mediator edges.
    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge>;

    /// Human-readable name for this pattern (used in logs).
    fn name(&self) -> &'static str;
}

/// Insert mediator edges into the graph as Configures edges.
/// Returns the count of successfully inserted edges.
pub fn apply_mediator(graph: &mut SymbolGraph, edges: Vec<MediatorEdge>) -> usize {
    let mut count = 0;
    // Convention-inferred: mediator edges are derived from framework patterns,
    // not from direct syntactic call sites.
    let origin = AnalysisOrigin::ConventionInferred;
    let confidence = EdgeEvaluator::evaluate(EdgeKind::Configures, origin);
    for e in edges {
        let edge = DirectEdge::new(
            e.from,
            e.to,
            EdgeKind::Configures,
            origin,
            confidence,
            e.evidence_file,
        );
        if graph.insert_edge(edge) {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolKind, SymbolNode};
    use std::path::PathBuf;

    #[test]
    fn apply_mediator_inserts_configures_edges() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserController",
            PathBuf::from("A.java"),
            0,
            10,
        ));
        let id_b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserService",
            PathBuf::from("B.java"),
            0,
            10,
        ));

        let edges = vec![MediatorEdge {
            from: id_a,
            to: id_b,
            evidence_file: PathBuf::from("A.java"),
        }];
        let count = apply_mediator(&mut g, edges);
        assert_eq!(count, 1);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn apply_mediator_empty_returns_zero() {
        let mut g = SymbolGraph::new();
        let count = apply_mediator(&mut g, vec![]);
        assert_eq!(count, 0);
    }
}

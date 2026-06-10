use crate::edge::DirectEdge;
use crate::symbol::{SymbolId, SymbolNode};

/// Minimal Vec-backed symbol graph used internally by `coregraph_core`.
///
/// Lookups are O(n) over the node vector. The production graph used by the
/// server is the petgraph-backed `coregraph_graph::SymbolGraph`; prefer that
/// type for any non-trivial consumer.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SymbolGraph {
    nodes: Vec<SymbolNode>,
    edges: Vec<DirectEdge>,
    next_id: u64,
}

impl SymbolGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a node and return its assigned SymbolId.
    pub fn insert_node(&mut self, mut node: SymbolNode) -> SymbolId {
        let id = SymbolId(self.next_id);
        self.next_id += 1;
        node.id = id;
        self.nodes.push(node);
        id
    }

    /// Insert a directed edge.
    pub fn insert_edge(&mut self, edge: DirectEdge) {
        self.edges.push(edge);
    }

    /// Look up a node by its SymbolId.
    pub fn get_node(&self, id: SymbolId) -> Option<&SymbolNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{AnalysisOrigin, Confidence, DirectEdge, EdgeKind};
    use crate::symbol::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn make_node(name: &str, kind: SymbolKind) -> SymbolNode {
        SymbolNode::new(SymbolId(0), kind, name, PathBuf::from("src/lib.rs"), 0, 10)
    }

    #[test]
    fn insert_and_retrieve_nodes() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("foo", SymbolKind::Function));
        let id_b = g.insert_node(make_node("Bar", SymbolKind::Class));

        assert_ne!(id_a, id_b);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.get_node(id_a).unwrap().name, "foo");
        assert_eq!(g.get_node(id_b).unwrap().name, "Bar");
    }

    #[test]
    fn insert_edge() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("caller", SymbolKind::Function));
        let id_b = g.insert_node(make_node("callee", SymbolKind::Function));

        let edge = DirectEdge::new(
            id_a,
            id_b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("src/lib.rs"),
        );
        g.insert_edge(edge);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn unknown_id_returns_none() {
        let g = SymbolGraph::new();
        assert!(g.get_node(SymbolId(999)).is_none());
    }
}

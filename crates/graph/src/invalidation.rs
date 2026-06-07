use std::collections::HashSet;
use std::path::Path;

use coregraph_core::file_state::NodeStatus;

use crate::change_tracking::ChangeBatch;
use crate::epoch::GraphEpoch;
use crate::snapshot::GraphSnapshot;
use crate::symbol_graph::SymbolGraph;

/// Removes all graph content (nodes + edges) sourced from a set of changed files.
/// Rebuilds the graph in-place via a filtered snapshot.
pub struct GraphInvalidator;

impl GraphInvalidator {
    /// Invalidate content from `changed_files`, bump the epoch, and return
    /// `(nodes_removed, new_epoch)`.
    pub fn invalidate(
        graph: &mut SymbolGraph,
        changed_files: &[impl AsRef<Path>],
        epoch: GraphEpoch,
    ) -> (usize, GraphEpoch) {
        let changed: HashSet<&Path> = changed_files.iter().map(|p| p.as_ref()).collect();

        let snap = GraphSnapshot::from_graph(graph, epoch);
        let nodes_before = snap.nodes.len();
        let next_id = snap.next_id;

        let filtered_nodes: Vec<_> = snap
            .nodes
            .into_iter()
            .filter(|n| !changed.contains(n.file.as_ref()))
            .collect();
        let filtered_edges: Vec<_> = snap
            .edges
            .into_iter()
            .filter(|e| !changed.contains(e.evidence_file.as_ref()))
            .collect();

        let nodes_removed = nodes_before - filtered_nodes.len();
        let new_epoch = epoch.next();

        let new_snap = GraphSnapshot {
            schema_version: crate::snapshot::SNAPSHOT_SCHEMA_VERSION,
            epoch: new_epoch,
            // In-memory rebuild only; built_at is meaningful for persisted
            // snapshots, not this transient filter step.
            built_at: std::time::SystemTime::UNIX_EPOCH,
            nodes: filtered_nodes,
            edges: filtered_edges,
            next_id,
        };
        let (new_graph, _) = new_snap.into_graph();
        *graph = new_graph;

        (nodes_removed, new_epoch)
    }

    /// Evidence-preserving invalidation per docs §7.Layer3:
    ///
    /// - Files in `batch.removed` have their defined nodes marked `Gone`.
    ///   Incoming structural edges are preserved so callers still see the
    ///   historical shape until GC (`gc_gone`) runs.
    /// - Files in `batch.changed` have their defined nodes marked `Stale`
    ///   and their outgoing evidence edges dropped. The next extractor pass
    ///   re-inserts fresh definitions/edges and flips the status back to
    ///   `Verified`.
    ///
    /// Returns `(nodes_marked_stale, nodes_marked_gone, new_epoch)`.
    pub fn invalidate_evidence_based(
        graph: &mut SymbolGraph,
        batch: &ChangeBatch,
        epoch: GraphEpoch,
    ) -> (usize, usize, GraphEpoch) {
        let mut stale = 0;
        for file in &batch.changed {
            stale += graph.mark_file_nodes(file, NodeStatus::Stale);
        }
        let mut gone = 0;
        for file in &batch.removed {
            gone += graph.mark_file_nodes(file, NodeStatus::Gone);
        }
        let new_epoch = epoch.next();
        (stale, gone, new_epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_node(g: &mut SymbolGraph, name: &str, file: &str) -> coregraph_core::SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from(file),
            0,
            10,
        ))
    }

    #[test]
    fn invalidates_nodes_from_changed_file() {
        let mut g = SymbolGraph::new();
        insert_node(&mut g, "foo", "src/foo.rs");
        insert_node(&mut g, "bar", "src/bar.rs");
        insert_node(&mut g, "baz", "src/foo.rs");

        let (removed, new_epoch) = GraphInvalidator::invalidate(
            &mut g,
            &[PathBuf::from("src/foo.rs")],
            GraphEpoch::zero(),
        );
        assert_eq!(removed, 2, "foo and baz should be removed");
        assert_eq!(g.node_count(), 1, "only bar remains");
        assert_eq!(new_epoch.0, 1);
    }

    #[test]
    fn invalidates_edges_by_evidence_file() {
        let mut g = SymbolGraph::new();
        let id_a = insert_node(&mut g, "a", "a.rs");
        let id_b = insert_node(&mut g, "b", "b.rs");
        let edge = DirectEdge::new(
            id_a,
            id_b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.8),
            PathBuf::from("a.rs"),
        );
        g.insert_edge(edge);
        assert_eq!(g.edge_count(), 1);

        GraphInvalidator::invalidate(&mut g, &[PathBuf::from("a.rs")], GraphEpoch::zero());
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn unchanged_files_preserved() {
        let mut g = SymbolGraph::new();
        insert_node(&mut g, "stable", "stable.rs");
        insert_node(&mut g, "changed", "changed.rs");

        let (removed, _) = GraphInvalidator::invalidate(
            &mut g,
            &[PathBuf::from("changed.rs")],
            GraphEpoch::zero(),
        );
        assert_eq!(removed, 1);
        assert_eq!(g.node_count(), 1);
        assert!(g.nodes().any(|n| n.name == "stable"));
    }

    #[test]
    fn no_changed_files_no_op() {
        let mut g = SymbolGraph::new();
        insert_node(&mut g, "a", "a.rs");
        insert_node(&mut g, "b", "b.rs");

        let (removed, epoch) =
            GraphInvalidator::invalidate(&mut g, &[] as &[PathBuf], GraphEpoch::zero());
        assert_eq!(removed, 0);
        assert_eq!(g.node_count(), 2);
        assert_eq!(epoch.0, 1, "epoch still increments on invalidation");
    }
}

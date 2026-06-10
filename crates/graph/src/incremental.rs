//! Selective fast-path graph mutation for single-file reindexing.

use crate::SymbolGraph;
use coregraph_core::{DirectEdge, SymbolNode};
use std::path::Path;

/// Telemetry counts returned by [`SymbolGraph::reindex_file_fast`].
#[derive(Debug, Clone, Default)]
pub struct FastReindexResult {
    pub nodes_removed: usize,
    pub nodes_inserted: usize,
    pub edges_removed: usize,
    /// Number of edges *submitted* for insertion (`new_edges.len()`), not the
    /// number confirmed inserted. `insert_edge`'s success/failure return is not
    /// consulted, so an edge built against a pre-insert id that fails to insert
    /// is still counted here.
    pub edges_inserted: usize,
    pub cross_file_edges_staled: usize,
}

impl SymbolGraph {
    /// Apply a fast-path reindex for a single file.
    ///
    /// Effects (in order):
    /// 1. Every node whose `file` equals `path` is removed (via `remove_node`,
    ///    which also drops incident edges and cleans indexes).
    /// 2. Edges with `evidence_file == path` are counted for telemetry (they
    ///    are dropped implicitly when their endpoint nodes are removed above).
    /// 3. `new_nodes` and `new_edges` are inserted into the graph. Note that
    ///    `insert_node` assigns fresh ids, so an edge in `new_edges` referencing
    ///    a pre-insert id may fail to insert; such failures are not detected and
    ///    are still counted in `FastReindexResult::edges_inserted`.
    /// 4. Any surviving edge whose `evidence_file` is NOT `path` but which has
    ///    one of the newly-inserted nodes as an endpoint has its
    ///    `stale_evidence_count` bumped by one — signals that this cross-file
    ///    edge's evidence may now be out of date.
    /// 5. `mark_fast_update(path)` records the per-file update instant.
    ///
    /// Returns telemetry counts describing the mutation. This method is not
    /// yet wired into the daemon's hot-update loop; the daemon currently
    /// performs its single-file fast reindex through a separate path
    /// (`reindex_file_in_place` in `cli/src/dispatch.rs`), so this method has
    /// no production callers today.
    pub fn reindex_file_fast(
        &mut self,
        path: &Path,
        new_nodes: Vec<SymbolNode>,
        new_edges: Vec<DirectEdge>,
    ) -> FastReindexResult {
        // Snapshot old node ids and edge count BEFORE mutation.
        let old_node_ids: Vec<_> = self
            .nodes()
            .filter(|n| n.file.as_ref() == path)
            .map(|n| n.id)
            .collect();
        let edges_removed = self.edges_for_file(path).count();
        let nodes_removed = old_node_ids.len();

        for id in &old_node_ids {
            self.remove_node(*id);
        }

        for n in new_nodes {
            self.insert_node(n);
        }
        let new_edges_count = new_edges.len();
        for e in new_edges {
            self.insert_edge(e);
        }

        // Collect ids of nodes just inserted for this file. We re-query the
        // graph rather than capturing pre-insert ids from the Vec because
        // `insert_node` assigns fresh ids regardless of the SymbolId in the
        // input node.
        let inserted_ids: std::collections::HashSet<_> = self
            .nodes()
            .filter(|n| n.file.as_ref() == path)
            .map(|n| n.id)
            .collect();

        // Bump stale_evidence_count on any edge whose evidence_file is NOT
        // the reindexed path but whose endpoint is in the newly-inserted set.
        // Under normal operation (insert_node assigns fresh ids) these will be
        // zero, because remove_node already dropped edges incident to the old
        // nodes. The bump fires when id recycling or an unusual reindex pattern
        // produces an endpoint collision with a surviving edge.
        let edges_to_bump: Vec<_> = self
            .edges()
            .filter(|e| e.evidence_file.as_ref() != path)
            .filter(|e| inserted_ids.contains(&e.from) || inserted_ids.contains(&e.to))
            .map(|e| (e.from, e.to, e.kind.clone()))
            .collect();
        let cross_file_edges_staled = edges_to_bump.len();
        for (from, to, kind) in edges_to_bump {
            self.bump_stale_evidence(from, to, kind);
        }

        self.mark_fast_update(path);

        FastReindexResult {
            nodes_removed,
            nodes_inserted: inserted_ids.len(),
            edges_removed,
            edges_inserted: new_edges_count,
            cross_file_edges_staled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    #[test]
    fn reindex_file_fast_drops_incident_edges_when_node_removed() {
        // Verifies the drop-via-remove_node path: when a.rs is reindexed, the
        // old a_fn node is removed, and the cross-file edge (b.rs → a.rs)
        // incident to that old node disappears as a side effect — NOT via
        // the stale-bump path. cross_file_edges_staled is 0 here because
        // no edge survives to be bumped.
        let mut g = SymbolGraph::new();

        // Seed: node in a.rs that receives a call FROM b.rs.
        let a_old = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a_fn",
            PathBuf::from("a.rs"),
            0,
            4,
        ));
        let b_node = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b_fn",
            PathBuf::from("b.rs"),
            0,
            4,
        ));
        // Cross-file edge: b.rs's b_fn calls a.rs's a_fn. Evidence is b.rs.
        g.insert_edge(DirectEdge::new(
            b_node,
            a_old,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("b.rs"),
        ));

        // Pre-condition: the cross-file edge exists with stale count 0.
        let pre = g
            .edges()
            .find(|e| e.from == b_node && e.to == a_old)
            .unwrap();
        assert_eq!(pre.stale_evidence_count, 0);

        // Reparse a.rs: it now defines a_fn at a new location.
        let new_nodes = vec![SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a_fn",
            PathBuf::from("a.rs"),
            10,
            14,
        )];

        let result = g.reindex_file_fast(&PathBuf::from("a.rs"), new_nodes, Vec::new());

        assert_eq!(result.nodes_removed, 1);
        assert_eq!(result.nodes_inserted, 1);
        // No surviving edge references the new node id, so nothing to bump.
        assert_eq!(
            result.cross_file_edges_staled, 0,
            "no edge survives with an endpoint matching the newly-inserted node"
        );

        // The edge disappeared because its endpoint (a_old) disappeared —
        // dropped by remove_node, not by any explicit edge cleanup.
        assert!(g
            .edges()
            .find(|e| e.from == b_node && e.to == a_old)
            .is_none());
    }

    #[test]
    fn reindex_file_fast_preserves_unrelated_file() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a_fn",
            PathBuf::from("a.rs"),
            0,
            4,
        ));
        let b_node = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b_fn",
            PathBuf::from("b.rs"),
            0,
            4,
        ));

        let result = g.reindex_file_fast(&PathBuf::from("a.rs"), Vec::new(), Vec::new());

        assert_eq!(result.nodes_removed, 1);
        assert_eq!(result.nodes_inserted, 0);
        // b.rs's node is untouched.
        assert!(g.get_node(b_node).is_some());
    }

    #[test]
    fn reindex_file_fast_marks_fast_update_time() {
        let mut g = SymbolGraph::new();
        assert!(g.last_fast_update_at(&PathBuf::from("a.rs")).is_none());

        g.reindex_file_fast(&PathBuf::from("a.rs"), Vec::new(), Vec::new());

        assert!(g.last_fast_update_at(&PathBuf::from("a.rs")).is_some());
    }

    #[test]
    fn reindex_file_fast_reports_edges_removed_count_accurately() {
        // Setup: two edges evidenced in a.rs plus one in b.rs. Reindex a.rs
        // with empty new_nodes/new_edges. The result should report
        // edges_removed == 2 (the two edges evidenced in a.rs), not including
        // the b.rs-evidenced edge (even though it may be dropped as a side
        // effect of remove_node because its endpoint lives in a.rs).
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "A",
            PathBuf::from("a.rs"),
            0,
            1,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "B",
            PathBuf::from("b.rs"),
            0,
            1,
        ));
        // Two edges evidenced in a.rs:
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::References,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        // One edge evidenced in b.rs:
        g.insert_edge(DirectEdge::new(
            b,
            a,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("b.rs"),
        ));

        let result = g.reindex_file_fast(&PathBuf::from("a.rs"), Vec::new(), Vec::new());

        // edges_removed counts ONLY edges evidenced in the reindexed file.
        assert_eq!(result.edges_removed, 2);
    }
}

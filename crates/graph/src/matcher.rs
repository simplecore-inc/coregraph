use crate::edge_evaluator::EdgeEvaluator;
use crate::symbol_graph::SymbolGraph;
use crate::value_index::ValueIndex;
use coregraph_core::edge::AnalysisOrigin;
use coregraph_core::{DirectEdge, EdgeKind};

/// Scans the SymbolGraph for string value matches and enum inconsistencies,
/// inserting StringMatch edges where applicable.
pub struct ValueMatcher;

impl ValueMatcher {
    /// Build a ValueIndex from the graph, then insert StringMatch edges
    /// for all cross-file matching string literals. `max_files_per_value`
    /// is forwarded to `ValueIndex::matching_string_pairs` (0 = unlimited).
    /// Returns the count of edges added.
    pub fn match_strings(graph: &mut SymbolGraph, max_files_per_value: usize) -> usize {
        let index = ValueIndex::build_from_graph(graph);
        let pairs = index.matching_string_pairs(graph, max_files_per_value);
        let mut count = 0;
        let origin = AnalysisOrigin::SyntaxMatched;
        let confidence = EdgeEvaluator::evaluate(EdgeKind::StringMatch, origin);
        for (from, to) in pairs {
            let evidence = graph
                .get_node(from)
                .map(|n| n.file.to_path_buf())
                .unwrap_or_default();
            let edge = DirectEdge::new(
                from,
                to,
                EdgeKind::StringMatch,
                origin,
                confidence,
                evidence,
            );
            if graph.insert_edge(edge) {
                count += 1;
            }
        }
        count
    }

    /// Returns enum variant names that appear in multiple enums.
    /// Does not modify the graph — caller decides what to do with results.
    pub fn detect_enum_mismatches(graph: &SymbolGraph) -> Vec<String> {
        ValueIndex::build_from_graph(graph).mismatched_variant_names()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_str_node(g: &mut SymbolGraph, name: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            name,
            PathBuf::from(file),
            0,
            name.len() as u32,
        ))
    }

    #[test]
    fn match_strings_inserts_edge() {
        let mut g = SymbolGraph::new();
        insert_str_node(&mut g, "/api/users", "client.ts");
        insert_str_node(&mut g, "/api/users", "Controller.java");

        let count = ValueMatcher::match_strings(&mut g, 0);
        assert_eq!(count, 1);
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn match_strings_no_cross_file_no_edges() {
        let mut g = SymbolGraph::new();
        insert_str_node(&mut g, "/api", "only.ts");
        insert_str_node(&mut g, "/other", "only.ts");

        let count = ValueMatcher::match_strings(&mut g, 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn detect_enum_mismatches_returns_variants() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::EnumVariant,
            "OrderStatus::Pending",
            PathBuf::from("a.java"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::EnumVariant,
            "PaymentStatus::Pending",
            PathBuf::from("b.ts"),
            0,
            10,
        ));

        let mismatches = ValueMatcher::detect_enum_mismatches(&g);
        assert!(mismatches.contains(&"Pending".to_string()));
    }
}

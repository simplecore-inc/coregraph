use super::{Mediator, MediatorEdge};
use crate::symbol_graph::SymbolGraph;
use coregraph_core::SymbolKind;

/// React Router mediator: detects route path → component bindings.
///
/// A StringLiteral node whose name starts with "/" (a route path) in the same
/// file as a PascalCase Function or Class node (a React component) within
/// 200 bytes is treated as a Route binding. Adds a Configures edge:
/// route_literal → component.
pub struct ReactRouterMediator;

impl Mediator for ReactRouterMediator {
    fn name(&self) -> &'static str {
        "react-router"
    }

    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge> {
        let mut edges = Vec::new();

        let routes: Vec<_> = graph
            .nodes()
            .filter(|n| n.kind == SymbolKind::StringLiteral && n.name.starts_with('/'))
            .map(|n| (n.id, n.file.clone(), n.span_start))
            .collect();

        for (route_id, route_file, route_span) in &routes {
            for node in graph.nodes() {
                if node.file != *route_file {
                    continue;
                }
                if node.kind != SymbolKind::Function && node.kind != SymbolKind::Class {
                    continue;
                }
                if !node.name.chars().next().is_some_and(|c| c.is_uppercase()) {
                    continue;
                }
                let distance = (node.span_start as i64 - *route_span as i64).unsigned_abs();
                if distance <= 200 {
                    edges.push(MediatorEdge {
                        from: *route_id,
                        to: node.id,
                        evidence_file: route_file.to_path_buf(),
                    });
                }
            }
        }
        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol_graph::SymbolGraph;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    #[test]
    fn detects_route_to_component_binding() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "/users",
            PathBuf::from("App.tsx"),
            10,
            16,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "UsersPage",
            PathBuf::from("App.tsx"),
            20,
            100,
        ));

        let detector = ReactRouterMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 1, "should link route to component");
    }

    #[test]
    fn no_edge_for_different_file() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "/dashboard",
            PathBuf::from("routes.tsx"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Dashboard",
            PathBuf::from("components.tsx"),
            0,
            50,
        ));

        let detector = ReactRouterMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 0, "cross-file route-component not matched");
    }

    #[test]
    fn no_edge_for_lowercase_function() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "/home",
            PathBuf::from("App.tsx"),
            0,
            5,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "handleRoute",
            PathBuf::from("App.tsx"),
            5,
            50,
        ));

        let detector = ReactRouterMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 0, "lowercase functions are not components");
    }

    #[test]
    fn no_edge_for_far_component() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "/far",
            PathBuf::from("App.tsx"),
            0,
            4,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "FarPage",
            PathBuf::from("App.tsx"),
            500,
            600,
        ));

        let detector = ReactRouterMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 0, "component too far from route path");
    }

    #[test]
    fn no_edge_for_component_far_before_route() {
        let mut g = SymbolGraph::new();
        // Component at span 0, route at span 500 — 500 bytes apart, should NOT match
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "EarlyPage",
            PathBuf::from("App.tsx"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "/early",
            PathBuf::from("App.tsx"),
            500,
            506,
        ));

        let detector = ReactRouterMediator;
        let edges = detector.detect(&g);
        assert_eq!(
            edges.len(),
            0,
            "component far before route should not match"
        );
    }
}

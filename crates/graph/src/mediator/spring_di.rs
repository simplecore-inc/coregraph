use super::{Mediator, MediatorEdge};
use crate::symbol_graph::SymbolGraph;
use coregraph_core::SymbolKind;

/// Spring DI mediator: detects constructor/field injection patterns.
///
/// When a Field or Variable node's name correlates with a Class node name in
/// another file, this indicates a Spring @Autowired-style injection and adds
/// a Configures edge. Matching rules (all case-insensitive):
///
/// 1. Exact match: field `userService` ↔ class `UserService`
/// 2. Suffix match: field `service` ↔ class `UserService` (min 4 chars)
/// 3. First-char-dropped: the class name minus its first character is matched
///    by Rule 1 or 2 (e.g. field `service` ↔ class `IUserService`). Because the
///    name is lowercased first, this fires for any class, not just Hungarian
///    `I`-prefixed interfaces.
pub struct SpringDiMediator;

/// Returns true if `field` is a plausible injection name for `class`.
fn is_injection_match(field: &str, class: &str) -> bool {
    if field.len() < 4 {
        return false;
    }
    let f = field.to_lowercase();
    let c = class.to_lowercase();
    // Rule 1: exact
    if f == c {
        return true;
    }
    // Rule 2: field is a trailing suffix of class (e.g. "service" ⊂ "userservice")
    if c.ends_with(&f) && c.len() > f.len() {
        return true;
    }
    // Rule 3: drop the class name's first character and retry Rule 1/2.
    // `c` is already lowercased, so this guard is true for any alphabetic class
    // name (not only Hungarian "IUserService" prefixes), which can over-match.
    if c.len() > 1 && c.chars().next().is_some_and(|ch| ch.is_ascii_lowercase()) {
        let trimmed = &c[1..];
        if trimmed == f || (trimmed.ends_with(&f) && trimmed.len() > f.len()) {
            return true;
        }
    }
    false
}

impl Mediator for SpringDiMediator {
    fn name(&self) -> &'static str {
        "spring-di"
    }

    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge> {
        // Collect class and interface nodes (Spring injects both).
        let classes: Vec<(coregraph_core::SymbolId, String, std::path::PathBuf)> = graph
            .nodes()
            .filter(|n| n.kind == SymbolKind::Class || n.kind == SymbolKind::Interface)
            .map(|n| (n.id, n.name.clone(), n.file.to_path_buf()))
            .collect();

        let mut edges = Vec::new();
        for node in graph.nodes() {
            if node.kind != SymbolKind::Field && node.kind != SymbolKind::Variable {
                continue;
            }
            for (class_id, class_name, class_file) in &classes {
                if node.file.as_ref() == class_file.as_path() {
                    continue;
                }
                if is_injection_match(&node.name, class_name) {
                    edges.push(MediatorEdge {
                        from: node.id,
                        to: *class_id,
                        evidence_file: node.file.to_path_buf(),
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
    fn detects_field_injection() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserService",
            PathBuf::from("Service.java"),
            0,
            50,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Field,
            "userService",
            PathBuf::from("Controller.java"),
            10,
            20,
        ));

        let detector = SpringDiMediator;
        let edges = detector.detect(&g);
        assert_eq!(
            edges.len(),
            1,
            "should detect userService → UserService injection"
        );
    }

    #[test]
    fn no_edge_for_same_file() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "Helper",
            PathBuf::from("app.java"),
            0,
            30,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Field,
            "helper",
            PathBuf::from("app.java"),
            10,
            20,
        ));

        let detector = SpringDiMediator;
        let edges = detector.detect(&g);
        assert_eq!(
            edges.len(),
            0,
            "same-file injection not a cross-file dependency"
        );
    }

    #[test]
    fn no_edge_when_no_class_match() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Field,
            "unknownDep",
            PathBuf::from("X.java"),
            0,
            10,
        ));

        let detector = SpringDiMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 0);
    }

    #[test]
    fn detects_suffix_injection() {
        // `UserService service;` is very common — field name is a suffix of class name.
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "UserService",
            PathBuf::from("Service.java"),
            0,
            50,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Field,
            "service",
            PathBuf::from("Controller.java"),
            10,
            20,
        ));

        let detector = SpringDiMediator;
        let edges = detector.detect(&g);
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn no_edge_for_too_short_field() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Class,
            "Logger",
            PathBuf::from("Log.java"),
            0,
            30,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Field,
            "log",
            PathBuf::from("C.java"),
            0,
            10,
        ));

        let detector = SpringDiMediator;
        let edges = detector.detect(&g);
        assert_eq!(
            edges.len(),
            0,
            "3-char fields should not match (too ambiguous)"
        );
    }

    #[test]
    fn is_injection_match_rules() {
        assert!(is_injection_match("userService", "UserService"));
        assert!(is_injection_match("service", "UserService"));
        assert!(is_injection_match("repository", "IRepository"));
        assert!(!is_injection_match("svc", "UserService"));
        assert!(!is_injection_match("unrelated", "UserService"));
    }
}

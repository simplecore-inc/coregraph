use coregraph_core::{EdgeKind, SymbolKind, SymbolNode};
use coregraph_graph::SymbolGraph;

/// Symbol kinds that represent code a dead-code analysis can meaningfully flag.
/// This is an ALLOW-list rather than a deny-list so that non-code kinds —
/// config keys, string literals, doc nodes, package/file/module containers —
/// can never be reported as "dead code" (they are values, manifests, or
/// structural containers, not code). Adding a new non-code `SymbolKind` is
/// therefore safe by default: it is excluded unless explicitly listed here.
/// Public-symbol heuristic for the `--public-only` orphan filter: a name is
/// "public" when it does not start with `_` and begins with a letter. Shared by
/// the CLI command and the daemon handler (`cached_orphans`) so both filter
/// identically — previously only the CLI applied it, so a warm daemon leaked
/// private symbols into a `--public-only` result.
pub fn is_public_name(name: &str) -> bool {
    !name.starts_with('_') && name.chars().next().is_some_and(|c| c.is_alphabetic())
}

/// Public-symbol test that prefers the extractor-declared `visibility` (the
/// root-cause signal: an actual `pub`/`export`/`public` keyword) and falls back
/// to the name-shape heuristic only when visibility is `Unknown` (e.g. Go/Python,
/// where the name convention IS the visibility).
pub fn is_public_symbol(node: &SymbolNode) -> bool {
    match node.visibility {
        coregraph_core::Visibility::Public => true,
        coregraph_core::Visibility::Private => false,
        coregraph_core::Visibility::Unknown => is_public_name(&node.name),
    }
}

fn is_code_kind(kind: &SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum
            // EnumVariant is intentionally excluded: enum members are used via
            // values()/valueOf/serialization/exhaustive match/reflection, none
            // of which create reference edges, so "no incoming edge" is not
            // reliable dead-code evidence (high false-positive rate).
            | SymbolKind::Constant
            | SymbolKind::Variable
            | SymbolKind::Field
            | SymbolKind::TypeAlias
            | SymbolKind::Namespace
    )
}

/// Returns all SymbolNodes that have no *semantic* edges (incoming or outgoing).
/// Purely structural edges (Contains/BelongsTo) that every definition
/// automatically participates in are ignored — a symbol wired up only by
/// its file's Contains edge and its module's BelongsTo edge is still
/// effectively orphaned in usage terms. `Documents` edges are ignored for the
/// same reason: a symbol having a doc comment does not make it *used*, so it
/// must not mask an otherwise-orphaned symbol.
pub fn find_orphans(graph: &SymbolGraph) -> Vec<SymbolNode> {
    let connected: std::collections::HashSet<coregraph_core::SymbolId> = graph
        .edges()
        .filter(|e| {
            !matches!(
                e.kind,
                EdgeKind::Contains
                    | EdgeKind::BelongsTo
                    | EdgeKind::Documents
                    | EdgeKind::DescribedIn
            )
        })
        .flat_map(|e| [e.from, e.to])
        .collect();

    graph
        .nodes()
        // Only code symbols can be "dead code". File/Module containers, config
        // keys, string literals, doc nodes, and package nodes are excluded by
        // the allow-list (see `is_code_kind`).
        .filter(|n| is_code_kind(&n.kind))
        .filter(|n| !connected.contains(&n.id))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_node(g: &mut SymbolGraph, name: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from("src/lib.rs"),
            0,
            10,
        ))
    }

    fn insert_edge(g: &mut SymbolGraph, from: SymbolId, to: SymbolId) {
        g.insert_edge(DirectEdge::new(
            from,
            to,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.8),
            PathBuf::from("src/lib.rs"),
        ));
    }

    #[test]
    fn no_edges_all_orphans() {
        let mut g = SymbolGraph::new();
        insert_node(&mut g, "a");
        insert_node(&mut g, "b");
        let orphans = find_orphans(&g);
        assert_eq!(orphans.len(), 2);
    }

    #[test]
    fn connected_nodes_not_orphans() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "a");
        let b = insert_node(&mut g, "b");
        insert_edge(&mut g, a, b);
        let orphans = find_orphans(&g);
        assert_eq!(orphans.len(), 0);
    }

    #[test]
    fn mixed_graph_partial_orphans() {
        let mut g = SymbolGraph::new();
        let a = insert_node(&mut g, "connected_a");
        let b = insert_node(&mut g, "connected_b");
        insert_node(&mut g, "orphan_c");
        insert_edge(&mut g, a, b);
        let orphans = find_orphans(&g);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].name, "orphan_c");
    }

    #[test]
    fn empty_graph_no_orphans() {
        let g = SymbolGraph::new();
        assert!(find_orphans(&g).is_empty());
    }

    #[test]
    fn enum_variants_are_not_orphans() {
        // Enum variants are used via values()/valueOf/serialization/exhaustive
        // match/reflection — none of which produce reference edges — so "no
        // incoming edge" is not reliable dead-code evidence. Flagging them
        // floods `orphans` with false positives (56 on the Exposed Kotlin repo),
        // so they are excluded across all languages.
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "dead_fn",
            PathBuf::from("a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::EnumVariant,
            "Color.RED",
            PathBuf::from("a.kt"),
            0,
            1,
        ));
        let names: Vec<String> = find_orphans(&g).iter().map(|o| o.name.clone()).collect();
        assert_eq!(
            names,
            vec!["dead_fn".to_string()],
            "enum variant must not be flagged as dead code, got {names:?}"
        );
    }

    #[test]
    fn non_code_kinds_are_never_orphans() {
        // Dead-code analysis must report only code symbols. Config keys, string
        // literals, and package nodes are values/manifests, not code — even with
        // zero edges they must not be flagged as "dead code".
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "dead_fn",
            PathBuf::from("a.rs"),
            0,
            10,
        ));
        for kind in [
            SymbolKind::ConfigKey,
            SymbolKind::StringLiteral,
            SymbolKind::ExternalPackage,
            SymbolKind::Package,
        ] {
            g.insert_node(SymbolNode::new(
                SymbolId(0),
                kind,
                "noncode",
                PathBuf::from("x"),
                0,
                1,
            ));
        }
        let names: Vec<String> = find_orphans(&g).iter().map(|o| o.name.clone()).collect();
        assert_eq!(
            names,
            vec!["dead_fn".to_string()],
            "only code symbols are dead-code orphans, got {names:?}"
        );
    }
}

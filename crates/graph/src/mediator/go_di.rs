use super::{Mediator, MediatorEdge};
use crate::symbol_graph::SymbolGraph;
use coregraph_core::SymbolKind;

/// Go DI mediator (Google Wire, Uber fx).
///
/// Scans Go files (.go) whose graph-extracted function symbols include
/// `wire.Build` or `fx.Provide` — the idiomatic DI wire-up points. In
/// such a file, collects every function whose name starts with `New` as a
/// provider; when there are at least two providers, emits a pairwise
/// lattice of Configures edges linking each provider to every other
/// provider in the file (so a file with fewer than two providers produces
/// no edges). The heuristic is coarse but matches what the
/// `SpringDiMediator` does for Spring: we surface plausible DI
/// relationships, not proven ones.
///
/// Refinement is deferred to grammar-level extraction of the individual
/// arguments inside `wire.Build(...)` / `fx.Provide(...)`.
pub struct GoDiMediator;

fn is_di_wireup(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower == "wire.build" || lower == "fx.provide" {
        return true;
    }
    // String literal extractor tags files with go_di_marker::wire.Build /
    // go_di_marker::fx.Provide so the mediator doesn't need source access.
    if let Some(tail) = lower.strip_prefix("go_di_marker::") {
        return tail == "wire.build" || tail == "fx.provide";
    }
    false
}

impl Mediator for GoDiMediator {
    fn name(&self) -> &'static str {
        "go-di"
    }

    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge> {
        // Index function nodes by file.
        let mut by_file: std::collections::HashMap<
            std::path::PathBuf,
            Vec<&coregraph_core::SymbolNode>,
        > = std::collections::HashMap::new();
        for n in graph.nodes() {
            let ext = n
                .file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "go" {
                continue;
            }
            if !matches!(n.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            by_file.entry(n.file.to_path_buf()).or_default().push(n);
        }

        let mut edges = Vec::new();
        for (file, nodes) in by_file {
            let has_wireup = nodes.iter().any(|n| is_di_wireup(&n.name));
            if !has_wireup {
                continue;
            }
            let providers: Vec<_> = nodes
                .iter()
                .filter(|n| n.name.starts_with("New") && n.name.len() > 3)
                .collect();
            if providers.len() < 2 {
                continue;
            }
            // Emit a lattice: every provider wires every other provider in
            // the same file — coarse but deterministic.
            for (i, a) in providers.iter().enumerate() {
                for b in providers.iter().skip(i + 1) {
                    edges.push(MediatorEdge {
                        from: a.id,
                        to: b.id,
                        evidence_file: file.clone(),
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
    use coregraph_core::{SymbolId, SymbolNode};
    use std::path::PathBuf;

    fn insert_fn(g: &mut SymbolGraph, name: &str, file: &str) -> SymbolId {
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
    fn wire_build_file_emits_provider_pairs() {
        let mut g = SymbolGraph::new();
        let a = insert_fn(&mut g, "NewStore", "wire.go");
        let b = insert_fn(&mut g, "NewService", "wire.go");
        let c = insert_fn(&mut g, "NewHandler", "wire.go");
        insert_fn(&mut g, "wire.Build", "wire.go");
        let edges = GoDiMediator.detect(&g);
        // Three providers → 3 choose 2 = 3 edges.
        assert_eq!(edges.len(), 3);
        assert!(edges.iter().any(|e| e.from == a && e.to == b));
        assert!(edges.iter().any(|e| e.from == a && e.to == c));
        assert!(edges.iter().any(|e| e.from == b && e.to == c));
    }

    #[test]
    fn fx_provide_recognized() {
        let mut g = SymbolGraph::new();
        let a = insert_fn(&mut g, "NewA", "main.go");
        let b = insert_fn(&mut g, "NewB", "main.go");
        insert_fn(&mut g, "fx.Provide", "main.go");
        let edges = GoDiMediator.detect(&g);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, a);
        assert_eq!(edges[0].to, b);
    }

    #[test]
    fn no_wire_no_edges() {
        let mut g = SymbolGraph::new();
        insert_fn(&mut g, "NewA", "plain.go");
        insert_fn(&mut g, "NewB", "plain.go");
        let edges = GoDiMediator.detect(&g);
        assert!(edges.is_empty());
    }

    #[test]
    fn non_go_files_ignored() {
        let mut g = SymbolGraph::new();
        insert_fn(&mut g, "NewA", "a.ts");
        insert_fn(&mut g, "NewB", "a.ts");
        insert_fn(&mut g, "wire.Build", "a.ts");
        let edges = GoDiMediator.detect(&g);
        assert!(edges.is_empty());
    }
}

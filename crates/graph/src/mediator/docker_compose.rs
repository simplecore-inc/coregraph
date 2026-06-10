use super::{Mediator, MediatorEdge};
use crate::symbol_graph::SymbolGraph;
use coregraph_core::SymbolKind;

/// docker-compose mediator.
///
/// `config_extractor` already emits ConfigKey nodes for every YAML dotted
/// path (`services.web`, `services.web.depends_on`, …). When a
/// `services.<A>.depends_on` key is present, this mediator emits Configures
/// edges from `services.<A>` to every *other* declared service in the same
/// file. Because the `depends_on` target name lives in the YAML value (which
/// is not indexed), the mediator cannot resolve the precise `services.<B>`
/// target and instead surfaces all sibling services as candidates — a coarse
/// upper bound rather than exact correspondences. This is the externally-
/// mediated pattern described in docs §5.7; precise value-side linking is
/// future work (see the inline note in `detect`).
///
/// We key off docker-compose.yml / docker-compose.yaml filenames to avoid
/// matching arbitrary YAML files that happen to have "services." prefixes.
pub struct DockerComposeMediator;

impl Mediator for DockerComposeMediator {
    fn name(&self) -> &'static str {
        "docker-compose"
    }

    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge> {
        // Index config keys by file + dotted name.
        let mut keys_by_file: std::collections::HashMap<
            std::path::PathBuf,
            Vec<&coregraph_core::SymbolNode>,
        > = std::collections::HashMap::new();
        for node in graph.nodes() {
            if node.kind != SymbolKind::ConfigKey {
                continue;
            }
            if !is_docker_compose_file(&node.file) {
                continue;
            }
            if node.name.starts_with("config_ref::") {
                continue;
            }
            keys_by_file
                .entry(node.file.to_path_buf())
                .or_default()
                .push(node);
        }

        let mut edges = Vec::new();
        for (file, nodes) in &keys_by_file {
            // Set of services declared in this file: top-level "services.<name>" keys.
            let services: std::collections::HashSet<String> = nodes
                .iter()
                .filter_map(|n| {
                    let trimmed = n.name.strip_prefix("services.")?;
                    // Only accept the direct child of `services.` — exactly one segment.
                    if trimmed.contains('.') {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect();
            if services.is_empty() {
                continue;
            }

            // For each "services.<A>.depends_on[.N]" key, link to "services.<A>" and
            // to the depended-upon service if it can be resolved.
            for node in nodes {
                let Some(tail) = node.name.strip_prefix("services.") else {
                    continue;
                };
                // Split on '.' — shape expected is `<A>.depends_on` or
                // `<A>.depends_on.<index>` or `<A>.environment.<key>`.
                let parts: Vec<&str> = tail.split('.').collect();
                if parts.len() < 2 {
                    continue;
                }
                let source_service = parts[0];
                let relation = parts[1];
                if relation != "depends_on" && relation != "depends-on" {
                    continue;
                }
                // Look for sibling service references. We don't know the
                // target-service name directly from the key (values aren't
                // indexed), so we synthesize an edge from source_service to
                // every other declared service. That's a coarse upper bound
                // — the mediator surfaces candidates; precise linking
                // requires value-side extraction that lives in future work.
                let Some(src_node) = nodes
                    .iter()
                    .find(|n| n.name == format!("services.{}", source_service))
                else {
                    continue;
                };
                for other in &services {
                    if other == source_service {
                        continue;
                    }
                    let Some(tgt_node) = nodes
                        .iter()
                        .find(|n| n.name == format!("services.{}", other))
                    else {
                        continue;
                    };
                    edges.push(MediatorEdge {
                        from: src_node.id,
                        to: tgt_node.id,
                        evidence_file: file.clone(),
                    });
                }
            }
        }
        edges
    }
}

pub(crate) fn is_docker_compose_file(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(
        name,
        "docker-compose.yml"
            | "docker-compose.yaml"
            | "compose.yml"
            | "compose.yaml"
            | "docker-compose.override.yml"
            | "docker-compose.override.yaml"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol_graph::SymbolGraph;
    use coregraph_core::{SymbolId, SymbolNode};
    use std::path::PathBuf;

    fn insert_key(g: &mut SymbolGraph, key: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            key,
            PathBuf::from(file),
            0,
            key.len() as u32,
        ))
    }

    #[test]
    fn filename_recognition() {
        assert!(is_docker_compose_file(&PathBuf::from("docker-compose.yml")));
        assert!(is_docker_compose_file(&PathBuf::from(
            "some/path/docker-compose.yaml"
        )));
        assert!(is_docker_compose_file(&PathBuf::from("compose.yml")));
        assert!(!is_docker_compose_file(&PathBuf::from("application.yml")));
    }

    #[test]
    fn depends_on_creates_service_edge() {
        let mut g = SymbolGraph::new();
        let file = "infra/docker-compose.yml";
        let web = insert_key(&mut g, "services.web", file);
        let db = insert_key(&mut g, "services.db", file);
        insert_key(&mut g, "services.web.depends_on", file);
        insert_key(&mut g, "services.web.depends_on.0", file);
        let edges = DockerComposeMediator.detect(&g);
        assert!(
            edges.iter().any(|e| e.from == web && e.to == db),
            "expected edge web → db, got {:?}",
            edges.iter().map(|e| (e.from, e.to)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_edge_in_non_compose_files() {
        let mut g = SymbolGraph::new();
        insert_key(&mut g, "services.web", "application.yml");
        insert_key(&mut g, "services.db", "application.yml");
        insert_key(&mut g, "services.web.depends_on", "application.yml");
        let edges = DockerComposeMediator.detect(&g);
        assert!(edges.is_empty());
    }
}

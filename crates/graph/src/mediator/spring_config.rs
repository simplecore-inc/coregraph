use super::{Mediator, MediatorEdge};
use crate::symbol_graph::SymbolGraph;
use coregraph_core::{SymbolId, SymbolKind};

/// Spring @Value / Spring Boot configuration mediator.
///
/// Connects `config_ref::<key>` ConfigKey nodes (emitted by
/// `string_literal_extractor` for `@Value("${foo.bar}")` and similar) to the
/// matching `<key>` ConfigKey definitions (emitted by `config_extractor`
/// from application.yml / application.toml). The mediator exists so the
/// graph carries explicit "this Java field reads this YAML key" edges even
/// when the reference and the definition live in different files — the
/// classical externally-mediated pattern from docs §5.7.
pub struct SpringConfigMediator;

const CONFIG_REF_PREFIX: &str = "config_ref::";

impl Mediator for SpringConfigMediator {
    fn name(&self) -> &'static str {
        "spring-config"
    }

    fn detect(&self, graph: &SymbolGraph) -> Vec<MediatorEdge> {
        // Build a case-insensitive index of ConfigKey definitions.
        let mut defs: std::collections::HashMap<String, (SymbolId, std::path::PathBuf)> =
            std::collections::HashMap::new();
        for node in graph.nodes() {
            if node.kind != SymbolKind::ConfigKey {
                continue;
            }
            if node.name.starts_with(CONFIG_REF_PREFIX) {
                continue;
            }
            defs.insert(
                node.name.to_ascii_lowercase(),
                (node.id, node.file.to_path_buf()),
            );
        }

        let mut edges = Vec::new();
        for node in graph.nodes() {
            if node.kind != SymbolKind::ConfigKey {
                continue;
            }
            let Some(key) = node.name.strip_prefix(CONFIG_REF_PREFIX) else {
                continue;
            };
            let Some((def_id, _)) = defs.get(&key.to_ascii_lowercase()) else {
                continue;
            };
            if *def_id == node.id {
                continue;
            }
            edges.push(MediatorEdge {
                from: node.id,
                to: *def_id,
                evidence_file: node.file.to_path_buf(),
            });
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

    fn insert_config_ref(g: &mut SymbolGraph, key: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            format!("{}{}", CONFIG_REF_PREFIX, key),
            PathBuf::from(file),
            0,
            key.len() as u32,
        ))
    }

    fn insert_config_def(g: &mut SymbolGraph, key: &str, file: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::ConfigKey,
            key.to_string(),
            PathBuf::from(file),
            0,
            key.len() as u32,
        ))
    }

    #[test]
    fn detects_java_value_yaml_binding() {
        let mut g = SymbolGraph::new();
        let ref_id = insert_config_ref(&mut g, "server.port", "AppConfig.java");
        let def_id = insert_config_def(&mut g, "server.port", "application.yml");
        let edges = SpringConfigMediator.detect(&g);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, ref_id);
        assert_eq!(edges[0].to, def_id);
    }

    #[test]
    fn no_edge_when_key_unknown() {
        let mut g = SymbolGraph::new();
        insert_config_ref(&mut g, "nonexistent.key", "AppConfig.java");
        insert_config_def(&mut g, "server.port", "application.yml");
        let edges = SpringConfigMediator.detect(&g);
        assert!(edges.is_empty());
    }

    #[test]
    fn case_insensitive_match() {
        let mut g = SymbolGraph::new();
        insert_config_ref(&mut g, "Server.Port", "AppConfig.java");
        insert_config_def(&mut g, "server.port", "application.yml");
        let edges = SpringConfigMediator.detect(&g);
        assert_eq!(edges.len(), 1);
    }
}

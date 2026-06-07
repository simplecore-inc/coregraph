use crate::symbol_graph::SymbolGraph;
use coregraph_core::{SymbolId, SymbolKind};
use std::collections::HashMap;

/// Maps string literal values to symbol IDs, and enum variant names to their owners.
/// Used by ValueMatcher to find cross-file string matches and enum inconsistencies.
pub struct ValueIndex {
    /// Normalized string value → list of SymbolIds containing that value
    string_values: HashMap<String, Vec<SymbolId>>,
    /// Variant name → list of (full_enum_name, SymbolId) pairs
    enum_variants: HashMap<String, Vec<(String, SymbolId)>>,
}

impl ValueIndex {
    /// Build an index from all StringLiteral and EnumVariant nodes in the graph.
    pub fn build_from_graph(graph: &SymbolGraph) -> Self {
        let mut idx = Self {
            string_values: HashMap::new(),
            enum_variants: HashMap::new(),
        };
        for node in graph.nodes() {
            match node.kind {
                SymbolKind::StringLiteral => {
                    idx.string_values
                        .entry(node.name.clone())
                        .or_default()
                        .push(node.id);
                }
                SymbolKind::EnumVariant => {
                    let variant = node
                        .name
                        .split("::")
                        .last()
                        .unwrap_or(&node.name)
                        .to_string();
                    idx.enum_variants
                        .entry(variant)
                        .or_default()
                        .push((node.name.clone(), node.id));
                }
                _ => {}
            }
        }
        idx
    }

    /// Returns pairs (from, to) of SymbolIds sharing the same string value
    /// but residing in different files. Also pairs enum variants with the
    /// same local name across different enum owners — these drive
    /// EnumValueMatch reclassification in the structural pass.
    pub fn matching_string_pairs(&self, graph: &SymbolGraph) -> Vec<(SymbolId, SymbolId)> {
        let mut pairs = Vec::new();
        // String literal pairs across files.
        for ids in self.string_values.values() {
            if ids.len() < 2 {
                continue;
            }
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    let file_a = graph.get_node(ids[i]).map(|n| &n.file);
                    let file_b = graph.get_node(ids[j]).map(|n| &n.file);
                    if let (Some(fa), Some(fb)) = (file_a, file_b) {
                        if fa != fb {
                            pairs.push((ids[i], ids[j]));
                        }
                    }
                }
            }
        }
        // Enum-variant pairs across distinct enums (same local name, different parent).
        for entries in self.enum_variants.values() {
            if entries.len() < 2 {
                continue;
            }
            for i in 0..entries.len() {
                for j in (i + 1)..entries.len() {
                    let (ref parent_a, id_a) = entries[i];
                    let (ref parent_b, id_b) = entries[j];
                    // Prefer a qualified parent (`Status.Active`). When the
                    // extractor only gave us the local variant name, fall
                    // back to the enclosing file stem so fixtures with
                    // single-name enum constants still classify.
                    let file_stem_a = graph
                        .get_node(id_a)
                        .and_then(|n| n.file.file_stem().and_then(|s| s.to_str()))
                        .unwrap_or("")
                        .to_string();
                    let file_stem_b = graph
                        .get_node(id_b)
                        .and_then(|n| n.file.file_stem().and_then(|s| s.to_str()))
                        .unwrap_or("")
                        .to_string();
                    let owner_a = parent_a
                        .rsplit_once('.')
                        .map(|(o, _)| o.to_string())
                        .or_else(|| parent_a.rsplit_once("::").map(|(o, _)| o.to_string()))
                        .unwrap_or(file_stem_a);
                    let owner_b = parent_b
                        .rsplit_once('.')
                        .map(|(o, _)| o.to_string())
                        .or_else(|| parent_b.rsplit_once("::").map(|(o, _)| o.to_string()))
                        .unwrap_or(file_stem_b);
                    if owner_a == owner_b {
                        continue;
                    }
                    // Only emit if the enum declarations live in different
                    // files. Same-file pairs are almost certainly intentional.
                    let fa = graph.get_node(id_a).map(|n| n.file.clone());
                    let fb = graph.get_node(id_b).map(|n| n.file.clone());
                    if let (Some(fa), Some(fb)) = (fa, fb) {
                        if fa == fb {
                            continue;
                        }
                    }
                    pairs.push((id_a, id_b));
                }
            }
        }
        pairs
    }

    /// Returns variant names that appear under more than one enum name
    /// (potential enum value inconsistency across languages/modules).
    pub fn mismatched_variant_names(&self) -> Vec<String> {
        self.enum_variants
            .iter()
            .filter(|(_, pairs)| {
                let unique_enums: std::collections::HashSet<&str> = pairs
                    .iter()
                    .map(|(full_name, _)| {
                        full_name.split("::").next().unwrap_or(full_name.as_str())
                    })
                    .collect();
                unique_enums.len() > 1
            })
            .map(|(variant, _)| variant.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::SymbolNode;
    use std::path::PathBuf;

    fn make_node(kind: SymbolKind, name: &str, file: &str) -> SymbolNode {
        SymbolNode::new(SymbolId(0), kind, name, PathBuf::from(file), 0, 10)
    }

    #[test]
    fn build_indexes_string_literals() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node(
            SymbolKind::StringLiteral,
            "/api/users",
            "client.ts",
        ));
        g.insert_node(make_node(
            SymbolKind::StringLiteral,
            "/api/users",
            "server.java",
        ));
        g.insert_node(make_node(SymbolKind::StringLiteral, "/other", "client.ts"));

        let idx = ValueIndex::build_from_graph(&g);
        let pairs = idx.matching_string_pairs(&g);
        assert_eq!(pairs.len(), 1, "should find one cross-file match");
    }

    #[test]
    fn same_file_strings_not_matched() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node(SymbolKind::StringLiteral, "/api", "client.ts"));
        g.insert_node(make_node(SymbolKind::StringLiteral, "/api", "client.ts"));

        let idx = ValueIndex::build_from_graph(&g);
        let pairs = idx.matching_string_pairs(&g);
        assert_eq!(pairs.len(), 0, "same-file matches should be excluded");
    }

    #[test]
    fn detects_enum_variant_mismatch() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node(
            SymbolKind::EnumVariant,
            "Status::Active",
            "java.java",
        ));
        g.insert_node(make_node(
            SymbolKind::EnumVariant,
            "UserState::Active",
            "go.go",
        ));

        let idx = ValueIndex::build_from_graph(&g);
        let mismatches = idx.mismatched_variant_names();
        assert!(mismatches.contains(&"Active".to_string()));
    }

    #[test]
    fn no_mismatch_when_same_enum() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node(
            SymbolKind::EnumVariant,
            "Status::Active",
            "a.java",
        ));
        g.insert_node(make_node(
            SymbolKind::EnumVariant,
            "Status::Inactive",
            "b.java",
        ));

        let idx = ValueIndex::build_from_graph(&g);
        let mismatches = idx.mismatched_variant_names();
        assert!(
            mismatches.is_empty(),
            "same enum in different files is not a mismatch"
        );
    }
}

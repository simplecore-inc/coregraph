use crate::types::{OutputConfig, OutputFormat, QueryResult};
use crate::OutputSerializer;

pub struct HumanSerializer;

impl OutputSerializer for HumanSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Human
    }

    fn serialize(&self, result: &QueryResult, config: &OutputConfig) -> String {
        let total_pages = if result.total_nodes == 0 {
            1
        } else {
            result.total_nodes.div_ceil(config.page_size)
        };
        let mut out = format!(
            "=== Query: {} (page {}/{}) ===\n",
            result.symbol_name,
            config.page + 1,
            total_pages,
        );
        out.push_str(&format!(
            "Total: {} symbols, {} edges\n\n",
            result.total_nodes, result.total_edges,
        ));
        if result.matches.is_empty() {
            out.push_str("(no matches)\n");
        } else {
            for node in &result.matches {
                out.push_str(&format!(
                    "  [{:?}] {} ({}:{}–{})\n",
                    node.kind,
                    node.name,
                    node.file.display(),
                    node.span_start,
                    node.span_end,
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OutputConfig, QueryResult};
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    #[test]
    fn human_contains_header() {
        let r = QueryResult::empty("Foo".to_string());
        let cfg = OutputConfig::default();
        let out = HumanSerializer.serialize(&r, &cfg);
        assert!(out.contains("Query: Foo"), "output: {out}");
    }

    #[test]
    fn human_shows_symbol_kind_and_name() {
        let mut r = QueryResult::empty("Bar".to_string());
        r.matches.push(SymbolNode::new(
            SymbolId(1u64),
            SymbolKind::Class,
            "MyClass",
            PathBuf::from("src/bar.rs"),
            1,
            10,
        ));
        r.total_nodes = 1;
        let out = HumanSerializer.serialize(&r, &OutputConfig::default());
        assert!(out.contains("MyClass"), "output: {out}");
        assert!(out.contains("Class"), "output: {out}");
    }

    #[test]
    fn human_format_returns_human() {
        assert_eq!(HumanSerializer.format(), OutputFormat::Human);
    }
}

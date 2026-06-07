use crate::types::{OutputConfig, OutputFormat, QueryResult};
use crate::OutputSerializer;

pub struct LlmSerializer;

impl OutputSerializer for LlmSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Llm
    }

    fn serialize(&self, result: &QueryResult, config: &OutputConfig) -> String {
        let total_pages = if result.total_nodes == 0 {
            1
        } else {
            result.total_nodes.div_ceil(config.page_size)
        };
        let header = format!(
            "<!-- coregraph: query={} page={}/{} -->\n",
            result.symbol_name,
            config.page + 1,
            total_pages,
        );
        let available = config.budget.available();
        // Reserve ~20 tokens for the continuation hint line
        const HINT_RESERVE: usize = 20;
        let content_budget = available.saturating_sub(HINT_RESERVE);

        let mut out = header;
        let mut used_tokens: usize = 0;
        let mut truncated = false;

        if result.matches.is_empty() {
            out.push_str("(no matches)\n");
        } else {
            for node in &result.matches {
                let line = format!("[{:?}] {}\n", node.kind, node.name);
                // Rough token estimate: 1 token ≈ 4 chars
                let line_tokens = line.len().div_ceil(4);
                if used_tokens + line_tokens > content_budget {
                    truncated = true;
                    break;
                }
                out.push_str(&line);
                used_tokens += line_tokens;
            }
        }

        if truncated {
            out.push_str(&format!(
                "\n<!-- token budget reached — request page {} to continue -->\n",
                config.page + 1,
            ));
        } else if config.page + 1 < total_pages {
            out.push_str(&format!(
                "\n<!-- {} more pages — request page {} to continue -->\n",
                total_pages - config.page - 1,
                config.page + 2,
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OutputConfig, OutputFormat, QueryResult};
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    #[test]
    fn llm_contains_page_header() {
        let s = LlmSerializer;
        let cfg = OutputConfig {
            page: 1,
            ..OutputConfig::default()
        };
        let r = QueryResult::empty("Foo".to_string());
        let out = s.serialize(&r, &cfg);
        assert!(out.contains("page"), "output: {out}");
    }

    #[test]
    fn llm_format_returns_llm() {
        assert_eq!(LlmSerializer.format(), OutputFormat::Llm);
    }

    #[test]
    fn llm_compact_no_redundant_paths() {
        let mut r = QueryResult::empty("Bar".to_string());
        r.matches.push(SymbolNode::new(
            SymbolId(1u64),
            SymbolKind::Function,
            "bar_fn",
            PathBuf::from("src/bar.rs"),
            10,
            50,
        ));
        let s = LlmSerializer;
        let cfg = OutputConfig::default();
        let out = s.serialize(&r, &cfg);
        assert!(out.contains("bar_fn"));
        assert!(out.contains("Function"));
    }

    #[test]
    fn llm_truncates_when_budget_exceeded() {
        use crate::types::TokenBudget;
        let mut r = QueryResult::empty("Baz".to_string());
        for i in 0..10 {
            r.matches.push(SymbolNode::new(
                SymbolId(i as u64),
                SymbolKind::Function,
                format!("fn_{i}"),
                PathBuf::from("src/baz.rs"),
                i as u32,
                (i + 1) as u32,
            ));
        }
        r.total_nodes = 10;
        let cfg = OutputConfig {
            budget: TokenBudget {
                max_tokens: 5,
                reserve_tokens: 0,
            },
            ..OutputConfig::default()
        };
        let out = LlmSerializer.serialize(&r, &cfg);
        assert!(
            out.contains("token budget reached"),
            "expected truncation hint in: {out}"
        );
        assert!(
            !out.contains("fn_9"),
            "should be truncated before fn_9: {out}"
        );
    }
}

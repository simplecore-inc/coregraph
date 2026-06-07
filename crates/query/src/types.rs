#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryResult {
    pub symbol_name: String,
    pub matches: Vec<coregraph_core::SymbolNode>,
    pub edges: Vec<coregraph_core::DirectEdge>,
    pub total_nodes: usize,
    pub total_edges: usize,
}

impl QueryResult {
    pub fn empty(symbol_name: String) -> Self {
        Self {
            symbol_name,
            matches: vec![],
            edges: vec![],
            total_nodes: 0,
            total_edges: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutputFormat {
    Human,
    Llm,
    Json,
}

#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub format: OutputFormat,
    pub budget: TokenBudget,
    pub page: usize,
    pub page_size: usize,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Human,
            budget: TokenBudget {
                max_tokens: 8000,
                reserve_tokens: 500,
            },
            page: 0,
            page_size: 50,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TokenBudget {
    pub max_tokens: usize,
    pub reserve_tokens: usize,
}

impl TokenBudget {
    pub fn available(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserve_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_available() {
        let budget = TokenBudget {
            max_tokens: 1000,
            reserve_tokens: 100,
        };
        assert_eq!(budget.available(), 900);
    }

    #[test]
    fn token_budget_saturating_sub() {
        let budget = TokenBudget {
            max_tokens: 50,
            reserve_tokens: 100,
        };
        assert_eq!(budget.available(), 0);
    }

    #[test]
    fn output_config_default_format() {
        let cfg = OutputConfig::default();
        assert_eq!(cfg.format, OutputFormat::Human);
    }

    #[test]
    fn query_result_empty() {
        let r = QueryResult::empty("Foo".to_string());
        assert_eq!(r.symbol_name, "Foo");
        assert!(r.matches.is_empty());
    }
}

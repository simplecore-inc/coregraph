use crate::global_opts::GlobalOpts;
use clap::Args;
use coregraph_extractor::build_graph;
use coregraph_graph::SymbolGraph;
use coregraph_query::query_symbol;
use serde::{Deserialize, Serialize};

#[derive(Args)]
pub struct BatchArgs {
    /// Path to a JSON file containing an array of query names.
    pub queries_file: String,
}

/// Result of a single batch query item.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct BatchResultItem {
    pub name: String,
    pub count: usize,
    pub symbols: Vec<String>,
}

/// Run a list of query names against the graph.
/// Returns one BatchResultItem per query name.
pub fn run_queries(names: &[String], graph: &SymbolGraph) -> Vec<BatchResultItem> {
    names
        .iter()
        .map(|name| {
            let result = query_symbol(name, graph);
            let symbols: Vec<String> = result.matches.iter().map(|n| n.name.clone()).collect();
            BatchResultItem {
                name: name.clone(),
                count: symbols.len(),
                symbols,
            }
        })
        .collect()
}

pub fn run(args: BatchArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(&args.queries_file)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", args.queries_file, e))?;
    let names: Vec<String> = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON in {}: {}", args.queries_file, e))?;

    let (graph, _) = build_graph(&globals.project)?;
    let results = run_queries(&names, &graph);
    let output = serde_json::to_string_pretty(&results)?;
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queries_returns_empty() {
        let graph = SymbolGraph::new();
        let results = run_queries(&[], &graph);
        assert!(results.is_empty());
    }

    #[test]
    fn query_against_empty_graph_returns_zero_count() {
        let graph = SymbolGraph::new();
        let names = vec!["main".to_string()];
        let results = run_queries(&names, &graph);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "main");
        assert_eq!(results[0].count, 0);
    }

    #[test]
    fn multiple_queries_all_returned() {
        let graph = SymbolGraph::new();
        let names = vec!["foo".to_string(), "bar".to_string(), "baz".to_string()];
        let results = run_queries(&names, &graph);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn batch_result_item_serializes() {
        let item = BatchResultItem {
            name: "test".to_string(),
            count: 0,
            symbols: vec![],
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"count\":0"));
    }
}

pub mod budget;
pub mod exclude;
pub mod impact;
pub mod inconsistencies;
pub mod library;
pub mod orphans;
pub mod ownership;
pub mod paginate;
pub mod serialize;
pub mod types;

pub use types::{OutputConfig, OutputFormat, QueryResult, TokenBudget};

/// Trait for formatting query results into different output formats.
pub trait OutputSerializer: Send + Sync {
    fn serialize(&self, result: &QueryResult, config: &OutputConfig) -> String;
    fn format(&self) -> OutputFormat;
}

pub use budget::{estimate_tokens, fits_budget};
pub use exclude::PathExcluder;
pub use impact::{
    compute_impact, compute_risk, is_test_path, is_test_symbol, is_test_symbol_in, path_confidence,
    BlastRadius, ImpactResult, ImpactRisk, RiskLevel, TestInfo,
};
pub use inconsistencies::{
    find_api_path_mismatches, find_config_key_mismatches, find_enum_mismatches,
    find_inconsistencies, InconsistencyCategory, InconsistencyReport,
};
pub use library::LibraryClassifier;
pub use orphans::{find_orphans, is_public_name, is_public_symbol};
pub use ownership::{blame_file, OwnershipInfo};
pub use paginate::paginate;
pub use serialize::{HumanSerializer, JsonSerializer, LlmSerializer};

/// Look up all symbols matching `name` in the graph (case-insensitive substring).
///
/// Uses the incrementally-maintained name_index on `SymbolGraph` so the
/// common exact-match case hits O(1) lookup; falls back to a fuzzy scan
/// over the index keys when no exact hit is found.
pub fn query_symbol(name: &str, graph: &coregraph_graph::SymbolGraph) -> QueryResult {
    // Exact-match fast path.
    let exact: Vec<_> = graph
        .lookup_by_name(name, usize::MAX)
        .into_iter()
        .cloned()
        .collect();
    let matches = if !exact.is_empty() {
        exact
    } else {
        graph
            .lookup_by_name_fuzzy(name, 1024)
            .into_iter()
            .cloned()
            .collect()
    };
    let total = matches.len();
    QueryResult {
        symbol_name: name.to_string(),
        matches,
        edges: vec![],
        total_nodes: total,
        total_edges: 0,
    }
}

/// Return aggregate statistics about the graph.
pub fn graph_stats(graph: &coregraph_graph::SymbolGraph) -> QueryResult {
    QueryResult {
        symbol_name: "<stats>".to_string(),
        matches: vec![],
        edges: vec![],
        total_nodes: graph.node_count(),
        total_edges: graph.edge_count(),
    }
}

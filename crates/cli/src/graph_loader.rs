//! Single entry point for "give me a project's fully-built graph."
//!
//! `commands/index.rs`, `commands/stats.rs`, `commands/impact.rs`, and
//! the daemon's `ProjectManager::get_or_load` closure all share this
//! canonical build path so the `stats` / `stats --breakdown` paths can't
//! silently diverge from one another.

use std::path::Path;

use coregraph_extractor::build_graph;
use coregraph_graph::SymbolGraph;

/// Build the full graph for `project_root` via the tree-sitter +
/// stack-graphs pipeline. Returns the graph plus the file count the
/// extractor pass saw (for `--stats` output).
pub fn load_project_graph(project_root: &Path) -> anyhow::Result<(SymbolGraph, usize)> {
    build_graph(project_root)
}

/// Same as `load_project_graph` but discards the file count, matching
/// the signature `ProjectManager::get_or_load` expects.
pub fn load_project_graph_only(project_root: &Path) -> anyhow::Result<SymbolGraph> {
    load_project_graph(project_root).map(|(g, _)| g)
}

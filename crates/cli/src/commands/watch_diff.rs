use crate::global_opts::GlobalOpts;
use clap::Args;
use coregraph_extractor::{all_extractors, build_graph, scanner};
use coregraph_graph::{
    FileStateTracker, GraphEpoch, GraphInvalidator, OnDemandHealer, SymbolGraph,
};
use coregraph_watcher::{FileWatcher, GitDiffStrategy};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct WatchArgs {
    /// Show diff between graph snapshots (before/after rebuild)
    #[arg(long, default_value_t = false)]
    pub diff: bool,

    /// Force full rebuild on each change instead of incremental invalidate+heal
    #[arg(long, default_value_t = false)]
    pub no_incremental: bool,
}

/// A unique key identifying a symbol across graph snapshots.
/// Two symbols with the same (name, file, kind) are considered the "same"
/// for diff purposes; span changes alone are not counted as diffs.
type SymbolKey = (String, PathBuf, coregraph_core::SymbolKind);

fn format_key(key: &SymbolKey) -> String {
    format!("{} [{:?}] @ {}", key.0, key.2, key.1.display())
}

fn snapshot_keys(graph: &SymbolGraph) -> std::collections::HashSet<SymbolKey> {
    graph
        .nodes()
        .map(|n| (n.name.clone(), n.file.to_path_buf(), n.kind.clone()))
        .collect()
}

/// Compute the symbolic diff between two graph states.
/// Returns (added, removed) where each entry is "name [Kind] @ file".
pub fn compute_diff(before: &SymbolGraph, after: &SymbolGraph) -> (Vec<String>, Vec<String>) {
    let before_keys = snapshot_keys(before);
    let after_keys = snapshot_keys(after);

    let added: Vec<String> = after_keys
        .difference(&before_keys)
        .map(format_key)
        .collect();
    let removed: Vec<String> = before_keys
        .difference(&after_keys)
        .map(format_key)
        .collect();

    (added, removed)
}

/// Run the built-in language extractors on a single file, appending results to `graph`.
/// Used by OnDemandHealer during incremental updates.
fn extract_one_file(path: &Path, graph: &mut SymbolGraph) {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    for extractor in all_extractors() {
        if scanner::extension_matches(path, extractor.file_extensions()) {
            let _ = extractor.extract(path, &source, graph);
            break;
        }
    }
}

pub fn run(args: WatchArgs, globals: &GlobalOpts) -> anyhow::Result<()> {
    let root_buf = globals.project_root();
    let root = root_buf.as_path();
    println!("Watching: {} (press Ctrl+C to stop)", root.display());

    // Build the initial graph snapshot.
    let (mut before, _) = build_graph(root)?;
    let mut epoch = GraphEpoch::zero();
    println!(
        "Initial index: {} symbols, {} edges",
        before.node_count(),
        before.edge_count()
    );

    // Filter out build outputs, dep caches, and user-configured exclude
    // patterns at the watcher boundary. Matches the daemon's behavior in
    // `server::run` so `coregraph watch --diff` and the background daemon
    // react to the same set of events.
    let excluder = coregraph_query::PathExcluder::from_project_root(root);
    let watcher = FileWatcher::with_filter(root, move |p| !excluder.is_excluded(p))?;

    // Layer-1: seed the content-hash tracker with the initial file set so
    // spurious notify events (touch without content change) are filtered.
    let mut file_states = FileStateTracker::new();
    let initial_files: std::collections::HashSet<PathBuf> =
        before.nodes().map(|n| n.file.to_path_buf()).collect();
    file_states.seed(initial_files.iter());

    loop {
        let paths = watcher.receiver.next_changed_files();
        if paths.is_empty() {
            continue;
        }
        // Canonicalize change-event paths so they match the node file paths
        // produced by the initial build.
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .collect();

        // Defensive: if a multi-file git operation is in progress, skip this batch —
        // the intermediate state is unreliable and git will fire another event on completion.
        if GitDiffStrategy::detect_git_operation(root) {
            println!("Git operation in progress, skipping rebuild");
            continue;
        }

        // Layer-1 filter: drop paths whose content hash is unchanged.
        let real_changes = file_states.filter_real_changes(paths.iter());
        if real_changes.is_empty() {
            println!("No content changes detected (hash match) — skipping.");
            continue;
        }

        // Combined list of paths we might care about (changed + removed).
        let mut candidates: Vec<PathBuf> = real_changes.changed.clone();
        candidates.extend(real_changes.removed.iter().cloned());

        // Filter to files this project cares about (source, config, etc.).
        let relevant: Vec<PathBuf> = candidates
            .into_iter()
            .filter(|p| {
                let extractors = all_extractors();
                extractors
                    .iter()
                    .any(|ex| scanner::extension_matches(p, ex.file_extensions()))
            })
            .collect();

        if relevant.is_empty() {
            continue;
        }

        println!(
            "Change detected in {} relevant file(s) — updating...",
            relevant.len()
        );

        let after = if args.no_incremental {
            // Full rebuild path.
            let (g, _) = build_graph(root)?;
            g
        } else {
            // Incremental: invalidate old content, then re-extract the changed files.
            let mut g = before.clone();
            // Layer-3: mark deleted files' nodes `Gone` so incoming structural
            // edges survive until the 5-minute GC window lapses (docs §7.Layer3).
            let relevant_removed: Vec<PathBuf> = real_changes
                .removed
                .iter()
                .filter(|p| {
                    let extractors = all_extractors();
                    extractors
                        .iter()
                        .any(|ex| scanner::extension_matches(p, ex.file_extensions()))
                })
                .cloned()
                .collect();
            let gone_batch = coregraph_graph::ChangeBatch {
                changed: Vec::new(),
                removed: relevant_removed,
            };
            let (_stale, gone, new_epoch) =
                GraphInvalidator::invalidate_evidence_based(&mut g, &gone_batch, epoch);
            epoch = new_epoch;
            if gone > 0 {
                println!(
                    "  layer-3: marked {} node(s) Gone (5-min GC TTL, epoch {})",
                    gone, epoch.0
                );
            }

            // Changed files: wipe + re-extract (the existing path). This
            // complements the Gone-marking above: removed files are kept as
            // tombstones, changed files are fully replaced.
            let changed_only: Vec<PathBuf> = real_changes
                .changed
                .iter()
                .filter(|p| relevant.iter().any(|r| r == *p))
                .cloned()
                .collect();
            let (removed, new_epoch) = GraphInvalidator::invalidate(&mut g, &changed_only, epoch);
            epoch = new_epoch;
            let added = OnDemandHealer::heal(&mut g, &changed_only, extract_one_file);
            println!(
                "  incremental: -{} symbols from changed files, +{} re-extracted (epoch {})",
                removed, added, epoch.0
            );
            g
        };

        if args.diff {
            let (added, removed) = compute_diff(&before, &after);
            if added.is_empty() && removed.is_empty() {
                println!("No symbol changes.");
            } else {
                for name in &added {
                    println!("  + {name}");
                }
                for name in &removed {
                    println!("  - {name}");
                }
            }
        } else {
            println!(
                "Rebuilt: {} symbols, {} edges",
                after.node_count(),
                after.edge_count()
            );
        }

        before = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn insert_node(g: &mut SymbolGraph, name: &str) -> SymbolId {
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from("src/lib.rs"),
            0,
            name.len() as u32,
        ))
    }

    #[test]
    fn diff_empty_graphs_no_changes() {
        let before = SymbolGraph::new();
        let after = SymbolGraph::new();
        let (added, removed) = compute_diff(&before, &after);
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn diff_added_symbol() {
        let before = SymbolGraph::new();
        let mut after = SymbolGraph::new();
        insert_node(&mut after, "new_fn");
        let (added, removed) = compute_diff(&before, &after);
        assert_eq!(added.len(), 1);
        assert!(added[0].contains("new_fn"));
        assert!(removed.is_empty());
    }

    #[test]
    fn diff_removed_symbol() {
        let mut before = SymbolGraph::new();
        insert_node(&mut before, "old_fn");
        let after = SymbolGraph::new();
        let (added, removed) = compute_diff(&before, &after);
        assert!(added.is_empty());
        assert_eq!(removed.len(), 1);
        assert!(removed[0].contains("old_fn"));
    }

    #[test]
    fn diff_same_symbols_no_changes() {
        let mut before = SymbolGraph::new();
        let mut after = SymbolGraph::new();
        insert_node(&mut before, "foo");
        insert_node(&mut after, "foo");
        let (added, removed) = compute_diff(&before, &after);
        assert!(added.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn diff_same_name_different_files_distinguished() {
        let mut before = SymbolGraph::new();
        before.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Config",
            PathBuf::from("a.rs"),
            0,
            6,
        ));
        let mut after = SymbolGraph::new();
        after.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Config",
            PathBuf::from("b.rs"),
            0,
            6,
        ));
        let (added, removed) = compute_diff(&before, &after);
        assert_eq!(added.len(), 1);
        assert_eq!(removed.len(), 1);
    }
}

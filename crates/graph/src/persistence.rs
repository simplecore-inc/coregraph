//! Daemon-side graph persistence: where a project's snapshot lives, how a
//! restored snapshot is admitted (or rejected as stale), and how a graph is
//! written atomically.
//!
//! This module is pure I/O + policy injection — it does NOT walk the source
//! tree itself (that needs `ignore`, a CLI-layer concern). The caller passes
//! an `accept` predicate that decides, given the snapshot's `built_at`,
//! whether the on-disk graph still matches the source. Keeping the walk out
//! of here lets the `server` crate and any embedder reuse persistence without
//! pulling in a filesystem-walk dependency.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::epoch::GraphEpoch;
use crate::snapshot::{load_snapshot, save_snapshot};
use crate::symbol_graph::SymbolGraph;

/// Canonical on-disk location of a project's daemon snapshot.
pub fn snapshot_path(project_root: &Path) -> PathBuf {
    project_root.join(".coregraph").join("snapshot.bin")
}

/// A graph restored from disk together with the metadata needed to keep
/// validating it.
pub struct WarmGraph {
    pub graph: SymbolGraph,
    pub epoch: GraphEpoch,
    /// The wall-clock time the persisted graph reflected its source tree.
    pub built_at: SystemTime,
}

/// Try to restore a graph from `path`.
///
/// Returns `Some` only when a snapshot exists, deserializes cleanly, AND
/// `accept(built_at)` returns `true` (the caller's staleness gate — typically
/// "no source file is newer than `built_at`"). Returns `None` when:
/// - the file is absent (cold start),
/// - it is corrupt or recorded under a different schema version (logged, then
///   treated as a rebuild signal rather than a hard error), or
/// - `accept` rejects it (source changed while the daemon was down).
///
/// In every `None` case the caller rebuilds from source, so a missing or
/// unusable snapshot never blocks a query.
pub fn warm_load(path: &Path, accept: impl FnOnce(SystemTime) -> bool) -> Option<WarmGraph> {
    if !path.exists() {
        return None;
    }
    match load_snapshot(path) {
        Ok((graph, epoch, built_at)) => {
            if accept(built_at) {
                Some(WarmGraph {
                    graph,
                    epoch,
                    built_at,
                })
            } else {
                None
            }
        }
        Err(e) => {
            // Corrupt / foreign-version / truncated snapshot. Not fatal: the
            // caller rebuilds. Log so the discard is visible rather than
            // silent.
            eprintln!(
                "[coregraph] ignoring unreadable snapshot {}: {} — rebuilding from source",
                path.display(),
                e
            );
            None
        }
    }
}

/// Atomically persist `graph` to `path`. Writes to a sibling temp file and
/// renames into place so a crash mid-write never leaves a half-written
/// snapshot that `warm_load` would later reject.
pub fn persist(
    path: &Path,
    graph: &SymbolGraph,
    epoch: GraphEpoch,
    built_at: SystemTime,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("bin.tmp");
    save_snapshot(&tmp, graph, epoch, built_at)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn one_node_graph(name: &str) -> SymbolGraph {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from("src/lib.rs"),
            0,
            10,
        ));
        g
    }

    #[test]
    fn persist_then_warm_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = snapshot_path(dir.path());
        let built_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

        persist(
            &path,
            &one_node_graph("foo"),
            GraphEpoch::zero().next(),
            built_at,
        )
        .unwrap();

        let warm = warm_load(&path, |_| true).expect("snapshot should load and be accepted");
        assert_eq!(warm.graph.node_count(), 1);
        assert_eq!(warm.epoch.0, 1);
        assert_eq!(warm.built_at, built_at);
    }

    #[test]
    fn warm_load_absent_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = snapshot_path(dir.path());
        assert!(warm_load(&path, |_| true).is_none());
    }

    #[test]
    fn warm_load_respects_accept_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let path = snapshot_path(dir.path());
        persist(
            &path,
            &one_node_graph("bar"),
            GraphEpoch::zero(),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100),
        )
        .unwrap();

        // accept() rejects (e.g. a newer source file was found) → None,
        // forcing the caller to rebuild.
        assert!(warm_load(&path, |_built_at| false).is_none());
    }

    #[test]
    fn persist_is_atomic_no_tmp_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = snapshot_path(dir.path());
        persist(
            &path,
            &one_node_graph("baz"),
            GraphEpoch::zero(),
            SystemTime::now(),
        )
        .unwrap();
        assert!(path.exists());
        assert!(
            !path.with_extension("bin.tmp").exists(),
            "temp file must be renamed away"
        );
    }
}

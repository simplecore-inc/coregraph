//! Layer 1 + Layer 3 of the 3-layer change tracking pipeline
//! (see `docs/change-tracking.md §7`).
//!
//! Layer 1 — `FileStateTracker`: filters notify events through a content-hash
//! check so identical writes (mtime bumps without real change) don't trigger
//! re-indexing.
//!
//! Layer 3 — `EvidenceIndex`: answers "which nodes and edges live in file
//! X" in O(1), so invalidation can scope to exactly what the change affects
//! while preserving the structural identity of surviving incoming edges.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use coregraph_core::file_state::{sample_file_state, FileState};
use coregraph_core::SymbolId;

/// Tracks the last-observed `FileState` per file so the watch pipeline can
/// drop spurious notify events where the content hash is unchanged.
#[derive(Debug, Default, Clone)]
pub struct FileStateTracker {
    states: HashMap<PathBuf, FileState>,
}

/// Outcome of re-sampling a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedChange {
    /// Content hash matches the previously-remembered value.
    Unchanged,
    /// File is new or its content hash differs.
    Changed,
    /// File was removed (e.g. `git rm`, `fs::remove_file`).
    Removed,
}

impl FileStateTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the tracked state for `path`, if any.
    pub fn get(&self, path: &Path) -> Option<&FileState> {
        self.states.get(path)
    }

    /// Seed the tracker with a batch of existing files. Used after an
    /// initial `build_graph` to avoid re-indexing on the first watch event.
    pub fn seed<I, P>(&mut self, files: I)
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        for p in files {
            if let Ok(state) = sample_file_state(p.as_ref()) {
                self.states.insert(p.as_ref().to_path_buf(), state);
            }
        }
    }

    /// Compare a filesystem sample against the last-observed state.
    /// Returns `Unchanged` for no-op writes (same hash).
    pub fn observe(&mut self, path: &Path) -> ObservedChange {
        match sample_file_state(path) {
            Ok(new_state) => {
                let previous = self.states.insert(path.to_path_buf(), new_state);
                match previous {
                    Some(prev) if prev.content_hash == new_state.content_hash => {
                        ObservedChange::Unchanged
                    }
                    _ => ObservedChange::Changed,
                }
            }
            Err(_) => {
                // File disappeared — remove it from the tracked set and
                // report a deletion.
                if self.states.remove(path).is_some() {
                    ObservedChange::Removed
                } else {
                    // We never knew about this file; treat as no-op.
                    ObservedChange::Unchanged
                }
            }
        }
    }

    /// Filter a batch of candidate paths through `observe`, returning only
    /// those that really changed or were removed.
    pub fn filter_real_changes<I, P>(&mut self, candidates: I) -> ChangeBatch
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        for p in candidates {
            let p = p.as_ref();
            match self.observe(p) {
                ObservedChange::Changed => changed.push(p.to_path_buf()),
                ObservedChange::Removed => removed.push(p.to_path_buf()),
                ObservedChange::Unchanged => {}
            }
        }
        ChangeBatch { changed, removed }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

/// Outcome of filtering a batch of candidate change notifications.
#[derive(Debug, Default, Clone)]
pub struct ChangeBatch {
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

impl ChangeBatch {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Per-file evidence pointers. Answers "what graph content lives in file X?"
/// so invalidation can scope to exactly what the change affects.
#[derive(Debug, Default, Clone)]
pub struct EvidenceSet {
    /// Symbols defined in this file.
    pub defined_nodes: HashSet<SymbolId>,
    /// Edges whose evidence_file points to this file. Edge identity is
    /// (from, to, kind-discriminant) — tracked as a stable key because edges
    /// in petgraph don't carry an external id.
    pub outgoing_edge_keys: HashSet<EdgeKey>,
}

/// Stable-across-rebuilds key for a directed edge (from, to, kind).
pub type EdgeKey = (SymbolId, SymbolId, u8);

/// Encode an `EdgeKind` variant into a compact discriminant so edges can be
/// keyed by `(from, to, kind)` without exposing the enum internals.
pub fn edge_kind_discriminant(kind: &coregraph_core::EdgeKind) -> u8 {
    use coregraph_core::EdgeKind;
    match kind {
        EdgeKind::Calls => 0,
        EdgeKind::References => 1,
        EdgeKind::Imports => 2,
        EdgeKind::Extends => 3,
        EdgeKind::Implements => 4,
        EdgeKind::Overrides => 5,
        EdgeKind::Resolves => 6,
        EdgeKind::StringMatch => 7,
        EdgeKind::Configures => 8,
        EdgeKind::DependsOn => 9,
        EdgeKind::Inherits => 10,
        EdgeKind::TypeOf => 11,
        EdgeKind::GenericParam => 12,
        EdgeKind::EnumValueMatch => 13,
        EdgeKind::ApiPathMatch => 14,
        EdgeKind::Contains => 15,
        EdgeKind::BelongsTo => 16,
        EdgeKind::Documents => 17,
        EdgeKind::Mentions => 18,
        EdgeKind::DescribedIn => 19,
    }
}

/// A file-indexed evidence map built eagerly from a `SymbolGraph`.
/// Used by `GraphInvalidator` and `OnDemandHealer` to scope invalidation.
#[derive(Debug, Default, Clone)]
pub struct EvidenceIndex {
    pub per_file: HashMap<PathBuf, EvidenceSet>,
}

impl EvidenceIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_node(&mut self, file: &Path, id: SymbolId) {
        self.per_file
            .entry(file.to_path_buf())
            .or_default()
            .defined_nodes
            .insert(id);
    }

    pub fn record_edge(&mut self, evidence_file: &Path, key: EdgeKey) {
        self.per_file
            .entry(evidence_file.to_path_buf())
            .or_default()
            .outgoing_edge_keys
            .insert(key);
    }

    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.per_file.keys().map(|p| p.as_path())
    }

    pub fn evidence_for(&self, file: &Path) -> Option<&EvidenceSet> {
        self.per_file.get(file)
    }

    pub fn remove_file(&mut self, file: &Path) -> Option<EvidenceSet> {
        self.per_file.remove(file)
    }

    /// Remove a specific symbol id from all per-file `defined_nodes` sets.
    /// Removes the file key entirely if both `defined_nodes` and
    /// `outgoing_edge_keys` become empty.
    pub fn remove_node(&mut self, id: SymbolId) {
        let mut empty_files: Vec<PathBuf> = Vec::new();
        for (file, set) in self.per_file.iter_mut() {
            set.defined_nodes.remove(&id);
            if set.defined_nodes.is_empty() && set.outgoing_edge_keys.is_empty() {
                empty_files.push(file.clone());
            }
        }
        for file in empty_files {
            self.per_file.remove(&file);
        }
    }

    /// Remove all edge keys that have `id` as either endpoint across every file.
    /// Returns count of keys removed (across all files).
    pub fn remove_edges_incident_to(&mut self, id: SymbolId) -> usize {
        let mut removed = 0;
        let mut empty_files: Vec<PathBuf> = Vec::new();
        for (file, set) in self.per_file.iter_mut() {
            let before = set.outgoing_edge_keys.len();
            set.outgoing_edge_keys
                .retain(|(from, to, _)| *from != id && *to != id);
            removed += before - set.outgoing_edge_keys.len();
            if set.defined_nodes.is_empty() && set.outgoing_edge_keys.is_empty() {
                empty_files.push(file.clone());
            }
        }
        for file in empty_files {
            self.per_file.remove(&file);
        }
        removed
    }
}

/// Tracks nodes marked `Gone` and the instant they were marked, so a later
/// GC pass can drop them after a grace period.
#[derive(Debug, Default, Clone)]
pub struct GoneCemetery {
    pub entries: HashMap<SymbolId, SystemTime>,
}

impl GoneCemetery {
    pub fn mark(&mut self, id: SymbolId) {
        self.entries.insert(id, SystemTime::now());
    }

    /// Return the subset of entries that have been Gone for at least `ttl`.
    pub fn ripe(&self, ttl: Duration, now: SystemTime) -> Vec<SymbolId> {
        self.entries
            .iter()
            .filter_map(|(id, when)| {
                let age = now.duration_since(*when).unwrap_or(Duration::ZERO);
                if age >= ttl {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn forget(&mut self, id: SymbolId) {
        self.entries.remove(&id);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Grace period before `Gone` nodes are reaped.
pub const GONE_GC_TTL: Duration = Duration::from_secs(5 * 60);

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(content).expect("write");
        f
    }

    #[test]
    fn tracker_reports_first_observation_as_changed() {
        let f = write_temp(b"hello");
        let mut t = FileStateTracker::new();
        assert_eq!(t.observe(f.path()), ObservedChange::Changed);
    }

    #[test]
    fn tracker_reports_identical_rewrite_as_unchanged() {
        let f = write_temp(b"hello");
        let mut t = FileStateTracker::new();
        t.observe(f.path());
        // Force an mtime bump while leaving content alone.
        std::fs::write(f.path(), b"hello").unwrap();
        assert_eq!(t.observe(f.path()), ObservedChange::Unchanged);
    }

    #[test]
    fn tracker_reports_content_change() {
        let f = write_temp(b"hello");
        let mut t = FileStateTracker::new();
        t.observe(f.path());
        std::fs::write(f.path(), b"goodbye").unwrap();
        assert_eq!(t.observe(f.path()), ObservedChange::Changed);
    }

    #[test]
    fn tracker_reports_removal() {
        let f = write_temp(b"hello");
        let path = f.path().to_path_buf();
        let mut t = FileStateTracker::new();
        t.observe(&path);
        drop(f);
        assert_eq!(t.observe(&path), ObservedChange::Removed);
    }

    #[test]
    fn evidence_index_records_and_queries() {
        let mut idx = EvidenceIndex::new();
        let p = PathBuf::from("src/main.rs");
        idx.record_node(&p, SymbolId(1));
        idx.record_edge(&p, (SymbolId(1), SymbolId(2), 0));
        let set = idx.evidence_for(&p).expect("must exist");
        assert!(set.defined_nodes.contains(&SymbolId(1)));
        assert_eq!(set.outgoing_edge_keys.len(), 1);
    }

    #[test]
    fn gone_cemetery_ripe_after_ttl() {
        let mut c = GoneCemetery::default();
        c.mark(SymbolId(5));
        // Treat "now" as 10 minutes in the future.
        let future = SystemTime::now() + Duration::from_secs(600);
        let ripe = c.ripe(Duration::from_secs(300), future);
        assert_eq!(ripe, vec![SymbolId(5)]);
    }

}

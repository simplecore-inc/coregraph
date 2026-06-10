use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::change_tracking::{FileStateTracker, ObservedChange};
use crate::symbol_graph::SymbolGraph;

/// Re-extracts symbols for changed files by calling a caller-supplied closure.
/// Does not depend on the extractor crate — extraction logic is injected.
pub struct OnDemandHealer;

impl OnDemandHealer {
    /// For each path in `changed_files` that exists on disk, call `extract_fn(path, graph)`.
    /// Returns the number of nodes added across all healed files.
    pub fn heal<F>(
        graph: &mut SymbolGraph,
        changed_files: &[impl AsRef<Path>],
        extract_fn: F,
    ) -> usize
    where
        F: Fn(&Path, &mut SymbolGraph),
    {
        let before = graph.node_count();
        for file in changed_files {
            let path = file.as_ref();
            if path.exists() {
                extract_fn(path, graph);
            }
        }
        graph.node_count().saturating_sub(before)
    }
}

/// Query-path healing — the piece that turns "`coregraph query X`" into a
/// point-in-time-correct view even when the watch loop hasn't caught up
/// yet. See docs/data-flow.md, "On-demand healing" under "Query response":
///
///   1. Collect evidence_file paths from the BFS over seed.
///   2. Check each path's content hash against FileStateTracker.
///   3. For files whose hash changed, run the provided extract closure
///      under a total wall-clock budget. Files whose healing exceeds the
///      budget are flagged in `stale_after_timeout`.
///   4. Return a HealingReport describing what happened.
pub struct QueryHealer {
    tracker: FileStateTracker,
    timeout: Duration,
    enabled: bool,
}

/// Outcome of a single query-path heal.
#[derive(Debug, Default, Clone)]
pub struct HealingReport {
    /// Files whose on-disk hash matched the remembered state — nothing to do.
    pub unchanged: Vec<PathBuf>,
    /// Files whose hash differed and were successfully re-parsed within budget.
    pub healed: Vec<PathBuf>,
    /// Files whose healing ran past the total budget. The graph still
    /// reflects the pre-heal state for these; callers should report
    /// `SemanticTrust::Unknown` on paths evidenced by them.
    pub stale_after_timeout: Vec<PathBuf>,
    /// Files that disappeared since the last observation.
    pub removed: Vec<PathBuf>,
}

impl HealingReport {
    pub fn is_empty(&self) -> bool {
        self.unchanged.is_empty()
            && self.healed.is_empty()
            && self.stale_after_timeout.is_empty()
            && self.removed.is_empty()
    }
}

impl QueryHealer {
    pub fn new(timeout: Duration) -> Self {
        Self {
            tracker: FileStateTracker::new(),
            timeout,
            enabled: true,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn seed<I, P>(&mut self, files: I)
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        self.tracker.seed(files);
    }

    /// Check `candidate_files` against the tracked hashes and re-extract
    /// those that actually changed. Stops re-parsing (marking the rest as
    /// stale-after-timeout) once the deadline elapses.
    pub fn heal_query<F>(
        &mut self,
        graph: &mut SymbolGraph,
        candidate_files: &[PathBuf],
        mut extract_fn: F,
    ) -> HealingReport
    where
        F: FnMut(&Path, &mut SymbolGraph),
    {
        let mut report = HealingReport::default();
        if !self.enabled {
            report.unchanged = candidate_files.to_vec();
            return report;
        }
        let deadline = Instant::now() + self.timeout;
        let mut deadline_hit = false;
        for path in candidate_files {
            match self.tracker.observe(path) {
                ObservedChange::Unchanged => report.unchanged.push(path.clone()),
                ObservedChange::Removed => report.removed.push(path.clone()),
                ObservedChange::Changed => {
                    if deadline_hit || Instant::now() >= deadline {
                        report.stale_after_timeout.push(path.clone());
                        continue;
                    }
                    extract_fn(path, graph);
                    if Instant::now() >= deadline {
                        deadline_hit = true;
                    }
                    report.healed.push(path.clone());
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn add_dummy_node(path: &Path, graph: &mut SymbolGraph) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        graph.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            path.to_path_buf(),
            0,
            10,
        ));
    }

    #[test]
    fn heal_calls_extract_for_existing_files() {
        let mut g = SymbolGraph::new();
        let existing = std::env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join("Cargo.toml").exists())
            .map(|p| p.join("Cargo.toml"))
            .unwrap();

        let count = OnDemandHealer::heal(&mut g, &[existing], add_dummy_node);
        assert_eq!(count, 1);
    }

    #[test]
    fn heal_skips_nonexistent_files() {
        let mut g = SymbolGraph::new();
        let count = OnDemandHealer::heal(
            &mut g,
            &[PathBuf::from("/nonexistent/phantom.rs")],
            add_dummy_node,
        );
        assert_eq!(count, 0);
    }

    #[test]
    fn heal_multiple_files() {
        let mut g = SymbolGraph::new();
        let existing = std::env::current_dir()
            .unwrap()
            .ancestors()
            .find(|p| p.join("Cargo.toml").exists())
            .map(|p| p.join("Cargo.toml"))
            .unwrap();

        let count = OnDemandHealer::heal(&mut g, &[existing.clone(), existing], add_dummy_node);
        assert_eq!(count, 2);
    }

    #[test]
    fn query_healer_flags_unchanged() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"hello").unwrap();
        let path = f.path().to_path_buf();
        let mut healer = QueryHealer::new(Duration::from_millis(200));
        healer.seed(std::iter::once(&path));
        let mut g = SymbolGraph::new();
        let report = healer.heal_query(&mut g, std::slice::from_ref(&path), |_, _| {});
        assert_eq!(report.unchanged.len(), 1);
        assert!(report.healed.is_empty());
    }

    #[test]
    fn query_healer_reruns_on_content_change() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"hello").unwrap();
        let path = f.path().to_path_buf();
        let mut healer = QueryHealer::new(Duration::from_millis(200));
        healer.seed(std::iter::once(&path));
        std::fs::write(&path, b"goodbye").unwrap();
        let mut g = SymbolGraph::new();
        let mut called_with = Vec::new();
        let report = healer.heal_query(&mut g, std::slice::from_ref(&path), |p, _| {
            called_with.push(p.to_path_buf());
        });
        assert_eq!(report.healed, vec![path]);
        assert_eq!(called_with.len(), 1);
    }

    #[test]
    fn query_healer_flags_removed_files() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"hi").unwrap();
        let path = f.path().to_path_buf();
        let mut healer = QueryHealer::new(Duration::from_millis(50));
        healer.seed(std::iter::once(&path));
        drop(f);
        let mut g = SymbolGraph::new();
        let report = healer.heal_query(&mut g, std::slice::from_ref(&path), |_, _| {});
        assert_eq!(report.removed, vec![path]);
    }

    #[test]
    fn query_healer_respects_timeout_budget() {
        use std::io::Write;
        // Two files both changed; extraction sleeps 120ms each with a 100ms budget.
        let mut a = tempfile::NamedTempFile::new().unwrap();
        let mut b = tempfile::NamedTempFile::new().unwrap();
        a.write_all(b"v1").unwrap();
        b.write_all(b"v1").unwrap();
        let pa = a.path().to_path_buf();
        let pb = b.path().to_path_buf();
        let mut healer = QueryHealer::new(Duration::from_millis(100));
        healer.seed([&pa, &pb]);
        std::fs::write(&pa, b"v2").unwrap();
        std::fs::write(&pb, b"v2").unwrap();
        let mut g = SymbolGraph::new();
        let report = healer.heal_query(&mut g, &[pa.clone(), pb.clone()], |_, _| {
            std::thread::sleep(Duration::from_millis(120));
        });
        assert_eq!(report.healed.len(), 1, "first heal completes");
        assert_eq!(
            report.stale_after_timeout.len(),
            1,
            "second heal flagged stale: {:?}",
            report
        );
    }

    #[test]
    fn query_healer_disabled_short_circuits() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"x").unwrap();
        let path = f.path().to_path_buf();
        let mut healer = QueryHealer::new(Duration::from_millis(200)).with_enabled(false);
        healer.seed(std::iter::once(&path));
        std::fs::write(&path, b"y").unwrap();
        let mut g = SymbolGraph::new();
        let mut called = false;
        let report = healer.heal_query(&mut g, std::slice::from_ref(&path), |_, _| {
            called = true;
        });
        assert!(!called, "disabled healer must not invoke extract");
        assert_eq!(report.unchanged.len(), 1);
    }

    // Regression: concurrent heal_query invocations from multiple threads
    // must not double-insert nodes or corrupt the FileStateTracker's
    // hash baseline. Reproduces the class of bug we saw in the daemon
    // before the heal_tracker seed fix: an unseeded tracker treats
    // every file as changed on the first call, and without external
    // synchronization two threads both re-extract the same file.
    #[test]
    fn query_healer_concurrent_reads_are_consistent() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use std::thread;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"v1").unwrap();
        let path = f.path().to_path_buf();

        let healer = Arc::new(Mutex::new(QueryHealer::new(Duration::from_millis(200))));
        healer.lock().unwrap().seed(std::iter::once(&path));

        let graph = Arc::new(Mutex::new(SymbolGraph::new()));
        let extract_calls = Arc::new(Mutex::new(0usize));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let healer = Arc::clone(&healer);
                let graph = Arc::clone(&graph);
                let counter = Arc::clone(&extract_calls);
                let p = path.clone();
                thread::spawn(move || {
                    let mut h = healer.lock().unwrap();
                    let mut g = graph.lock().unwrap();
                    h.heal_query(&mut g, std::slice::from_ref(&p), |_, _| {
                        *counter.lock().unwrap() += 1;
                    })
                })
            })
            .collect();

        let reports: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // All 8 threads saw the same file hash — the tracker should
        // report "unchanged" every time. Exactly zero extracts should
        // have run.
        for r in &reports {
            assert_eq!(r.unchanged.len(), 1, "report: {:?}", r);
            assert!(r.healed.is_empty());
            assert!(r.stale_after_timeout.is_empty());
        }
        assert_eq!(
            *extract_calls.lock().unwrap(),
            0,
            "extract closure must not run when the hash matches"
        );
    }

    // Regression: unseeded QueryHealer + concurrent threads — the race
    // we'd see without the daemon's startup seed. Only the first thread
    // to grab the tracker mutex should mark the file as changed; all
    // subsequent calls must treat it as unchanged.
    #[test]
    fn query_healer_unseeded_converges_after_first_observation() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use std::thread;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"bootstrap").unwrap();
        let path = f.path().to_path_buf();

        let healer = Arc::new(Mutex::new(QueryHealer::new(Duration::from_millis(200))));
        let graph = Arc::new(Mutex::new(SymbolGraph::new()));
        let extract_calls = Arc::new(Mutex::new(0usize));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let healer = Arc::clone(&healer);
                let graph = Arc::clone(&graph);
                let counter = Arc::clone(&extract_calls);
                let p = path.clone();
                thread::spawn(move || {
                    let mut h = healer.lock().unwrap();
                    let mut g = graph.lock().unwrap();
                    h.heal_query(&mut g, std::slice::from_ref(&p), |_, _| {
                        *counter.lock().unwrap() += 1;
                    })
                })
            })
            .collect();

        let _ = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>();

        // Only the first thread through the critical section records the
        // file as "healed"; the remaining seven must see it as
        // unchanged. Exactly one extract invocation total.
        assert_eq!(
            *extract_calls.lock().unwrap(),
            1,
            "exactly one thread should run the extract closure"
        );
    }

    // Regression: content mutation between seed and query must produce
    // exactly one heal event, not one per waiting thread.
    #[test]
    fn query_healer_content_change_heals_once_under_contention() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};
        use std::thread;

        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"before").unwrap();
        let path = f.path().to_path_buf();

        let healer = Arc::new(Mutex::new(QueryHealer::new(Duration::from_millis(200))));
        healer.lock().unwrap().seed(std::iter::once(&path));

        // Mutate the file so the first observer sees a hash mismatch.
        std::fs::write(&path, b"after").unwrap();

        let graph = Arc::new(Mutex::new(SymbolGraph::new()));
        let extract_calls = Arc::new(Mutex::new(0usize));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let healer = Arc::clone(&healer);
                let graph = Arc::clone(&graph);
                let counter = Arc::clone(&extract_calls);
                let p = path.clone();
                thread::spawn(move || {
                    let mut h = healer.lock().unwrap();
                    let mut g = graph.lock().unwrap();
                    h.heal_query(&mut g, std::slice::from_ref(&p), |_, _| {
                        *counter.lock().unwrap() += 1;
                    })
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            *extract_calls.lock().unwrap(),
            1,
            "exactly one thread re-extracts after the content change"
        );
    }
}

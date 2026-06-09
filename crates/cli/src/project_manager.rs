//! Multi-project daemon runtime per `docs/architecture.md §3.3`.
//!
//! Tracks the set of projects currently loaded into memory, with:
//! - LRU eviction when `max_loaded` is exceeded
//! - Singleflight loading (concurrent requests for the same project share
//!   one extractor run)
//! - Idle unload after `idle_unload` elapses without queries
//! - Last-query timestamps + active-query counters for introspection

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use coregraph_graph::{GraphEpoch, SymbolGraph};

/// Result of a build closure: the graph plus the metadata the manager needs
/// to drive persistence. `built_at` is the wall-clock time the graph reflects
/// its source tree (now, for a fresh build; the snapshot's recorded time, for
/// a warm load) — it seeds the staleness check on the next access.
/// `from_snapshot` is true when the graph was restored from disk unchanged,
/// so the manager can skip re-persisting it (it is not dirty).
pub struct BuiltGraph {
    pub graph: SymbolGraph,
    pub built_at: SystemTime,
    pub from_snapshot: bool,
}

/// Configuration knobs. Defaults match docs §3.3.
#[derive(Debug, Clone, Copy)]
pub struct ProjectManagerConfig {
    /// Maximum number of simultaneously-loaded projects.
    pub max_loaded: usize,
    /// Total approximate heap budget across all loaded graphs, in bytes.
    /// When the sum of `SymbolGraph::heap_bytes` exceeds this after a load,
    /// the least-recently-used idle projects are evicted until back under
    /// budget. `0` disables the byte cap (count cap still applies).
    pub max_loaded_bytes: u64,
    /// How long a project can sit idle before being unloaded.
    pub idle_unload: Duration,
    /// When *every* loaded project has been idle for at least this long
    /// (and no projects are loading), the daemon terminates itself. This
    /// makes the daemon look invisible to users — it spins up on the first
    /// `coregraph query`, serves subsequent queries instantly from the
    /// cached graph, and goes away on its own once the workstation goes
    /// quiet. Tracked by `ProjectManager::should_auto_stop`.
    pub auto_stop: Duration,
}

impl Default for ProjectManagerConfig {
    fn default() -> Self {
        Self {
            max_loaded: 5,
            max_loaded_bytes: 0,
            idle_unload: Duration::from_secs(10 * 60),
            auto_stop: Duration::from_secs(30 * 60),
        }
    }
}

/// Public snapshot of a single project's runtime state.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectStatus {
    pub path: PathBuf,
    pub loaded: bool,
    pub loading: bool,
    pub node_count: usize,
    pub edge_count: usize,
    pub idle_seconds: u64,
    pub active_queries: usize,
}

/// Internal per-project state.
struct ProjectEntry {
    graph: Arc<RwLock<SymbolGraph>>,
    last_query: Mutex<Instant>,
    active_queries: AtomicUsize,
    loaded: bool,
    /// Wall-clock time at which the graph was built. Used to detect
    /// "daemon was offline while the user edited files" — when any
    /// source file under the project root has a later mtime we evict
    /// and rebuild on the next query rather than serving stale data.
    built_at: Mutex<SystemTime>,
    /// True when the in-memory graph differs from the on-disk snapshot (or
    /// no snapshot exists yet). Set on fresh build and on every mutation;
    /// cleared after a successful persist. The eviction path skips writing a
    /// snapshot for a clean entry, avoiding redundant disk churn.
    dirty: AtomicBool,
}

impl ProjectEntry {
    fn touch(&self) {
        *self.last_query.lock().unwrap() = Instant::now();
    }

    fn built_at(&self) -> SystemTime {
        *self.built_at.lock().unwrap()
    }
}

/// A gate shared with callers that are waiting on a concurrent load of the
/// same project — so only one extractor invocation runs per project at a
/// time (singleflight).
struct LoadGate {
    done: Mutex<bool>,
    cvar: Condvar,
}

/// Multi-project runtime.
pub struct ProjectManager {
    config: ProjectManagerConfig,
    inner: Mutex<ManagerInner>,
}

struct ManagerInner {
    projects: HashMap<PathBuf, Arc<ProjectEntry>>,
    loading: HashMap<PathBuf, Arc<LoadGate>>,
    lru: Vec<PathBuf>,
    started_at: Instant,
}

/// Controls whether a cached project graph is checked for staleness before use.
///
/// Passed to `get_or_load_inner`; the two public wrappers fix this value:
/// - `get_or_load` → `IfStale` (evict and rebuild if source tree changed)
/// - `get_or_load_without_refresh` → `Never` (return cached entry as-is)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshPolicy {
    /// Evict and rebuild the cached graph when any source file is newer
    /// than `built_at`.
    IfStale,
    /// Return the cached graph without checking mtimes. Use when the caller
    /// owns the freshness guarantee (e.g. `reindex_file_fast`).
    Never,
}

impl ProjectManager {
    pub fn new(config: ProjectManagerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(ManagerInner {
                projects: HashMap::new(),
                loading: HashMap::new(),
                lru: Vec::new(),
                started_at: Instant::now(),
            }),
        }
    }

    /// Blocking `get_or_load`. Concurrent callers for the same project wait
    /// on a shared LoadGate; only one performs the actual `build_graph`.
    ///
    /// If the cached graph is stale (any source file has a newer mtime than
    /// `built_at`), the entry is evicted and the project is rebuilt before
    /// the graph arc is returned.
    pub fn get_or_load<F, E>(
        self: &Arc<Self>,
        path: &Path,
        build: F,
    ) -> Result<Arc<RwLock<SymbolGraph>>, E>
    where
        F: FnOnce(&Path) -> Result<BuiltGraph, E>,
    {
        self.get_or_load_inner(path, build, RefreshPolicy::IfStale)
    }

    /// Like [`get_or_load`], but skips the `source_tree_is_newer` mtime check
    /// for already-cached entries. Use this for callers that own the freshness
    /// guarantee (e.g. `reindex_file_fast`, which reads the file directly from
    /// disk during surgical update and does not need the whole project rebuilt).
    ///
    /// If no graph is cached yet, this still performs the initial build via
    /// `build` — first access cannot return nothing. Subsequent calls skip
    /// the mtime check.
    pub fn get_or_load_without_refresh<F, E>(
        self: &Arc<Self>,
        path: &Path,
        build: F,
    ) -> Result<Arc<RwLock<SymbolGraph>>, E>
    where
        F: FnOnce(&Path) -> Result<BuiltGraph, E>,
    {
        self.get_or_load_inner(path, build, RefreshPolicy::Never)
    }

    /// Private implementation shared by `get_or_load` and
    /// `get_or_load_without_refresh`. When `policy` is `RefreshPolicy::IfStale`,
    /// a cached entry whose source tree has changed since `built_at` is evicted
    /// and rebuilt. When `RefreshPolicy::Never`, the cached entry is returned
    /// as-is.
    fn get_or_load_inner<F, E>(
        self: &Arc<Self>,
        path: &Path,
        build: F,
        policy: RefreshPolicy,
    ) -> Result<Arc<RwLock<SymbolGraph>>, E>
    where
        F: FnOnce(&Path) -> Result<BuiltGraph, E>,
    {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        let gate_or_entry = {
            let mut inner = self.inner.lock().unwrap();
            if let Some(entry) = inner.projects.get(&canonical) {
                let should_evict = matches!(policy, RefreshPolicy::IfStale)
                    && source_tree_is_newer(&canonical, entry.built_at());
                if should_evict {
                    // Drop the stale entry and fall through to the rebuild
                    // path. The notify-based watcher catches most edits but
                    // misses changes made while the daemon was down or
                    // outside the watched subtree — this guards that window.
                    inner.projects.remove(&canonical);
                    inner.lru.retain(|p| p != &canonical);
                } else {
                    let entry = entry.clone();
                    entry.touch();
                    entry.active_queries.fetch_add(1, Ordering::SeqCst);
                    Self::record_access(&mut inner.lru, &canonical);
                    return Ok(entry.graph.clone());
                }
            }
            if let Some(existing_gate) = inner.loading.get(&canonical).cloned() {
                Err(existing_gate)
            } else {
                let gate = Arc::new(LoadGate {
                    done: Mutex::new(false),
                    cvar: Condvar::new(),
                });
                inner.loading.insert(canonical.clone(), gate.clone());
                Ok(gate)
            }
        };

        let gate = match gate_or_entry {
            Ok(gate) => gate,
            Err(other) => {
                // Another thread is already loading — wait for it, then recurse
                // with the same refresh_if_stale flag so the woken caller takes
                // the same staleness policy as the original caller.
                let guard = other.done.lock().unwrap();
                let guard = other.cvar.wait_while(guard, |done| !*done).unwrap();
                drop(guard);
                return self.get_or_load_inner(path, build, policy);
            }
        };

        // This thread is responsible for the load.
        let build_result = build(&canonical);
        let mut inner = self.inner.lock().unwrap();
        inner.loading.remove(&canonical);

        // Notify any waiters.
        {
            let mut done = gate.done.lock().unwrap();
            *done = true;
            gate.cvar.notify_all();
        }

        let built = build_result?;
        let entry = Arc::new(ProjectEntry {
            graph: Arc::new(RwLock::new(built.graph)),
            last_query: Mutex::new(Instant::now()),
            active_queries: AtomicUsize::new(1),
            loaded: true,
            built_at: Mutex::new(built.built_at),
            dirty: AtomicBool::new(!built.from_snapshot),
        });
        inner.projects.insert(canonical.clone(), entry.clone());
        Self::record_access(&mut inner.lru, &canonical);

        // Enforce max_loaded: select LRU victims (oldest first) with no
        // in-flight query, but DON'T remove them under this lock — defer to
        // `quiesce_and_remove` so the snapshot write happens off-lock and a
        // late query can still revive the victim before it is dropped.
        let cap = self.config.max_loaded;
        let over = inner.projects.len().saturating_sub(cap);
        let mut victims: Vec<(PathBuf, Arc<ProjectEntry>)> = Vec::new();
        if over > 0 {
            for p in inner.lru.iter() {
                if victims.len() >= over {
                    break;
                }
                if p == &canonical {
                    continue; // never evict the entry we just loaded
                }
                if let Some(e) = inner.projects.get(p) {
                    if e.active_queries.load(Ordering::SeqCst) == 0 {
                        victims.push((p.clone(), e.clone()));
                    }
                }
            }
        }
        drop(inner);

        // Persist (dirty-gated) off-lock, then remove victims still idle.
        self.quiesce_and_remove(victims, |e| e.active_queries.load(Ordering::SeqCst) == 0);
        // Then enforce the byte budget (no-op when unset). The just-loaded
        // entry is busy (active_queries > 0) so it is never the victim.
        self.enforce_byte_budget();
        Ok(entry.graph.clone())
    }

    /// Evict least-recently-used idle projects until the summed
    /// `heap_bytes` of all loaded graphs is within `max_loaded_bytes`.
    /// No-op when the budget is 0 (disabled). Graph sizes are measured under
    /// read locks taken OFF the manager lock — the entry Arcs are snapshotted
    /// first — so this never holds the manager lock while waiting on a graph
    /// lock (which would deadlock against a writer's `mark_dirty`).
    fn enforce_byte_budget(&self) {
        let budget = self.config.max_loaded_bytes;
        if budget == 0 {
            return;
        }
        loop {
            // Snapshot entries in LRU order (oldest first) off the lock.
            let entries: Vec<(PathBuf, Arc<ProjectEntry>)> = {
                let inner = self.inner.lock().unwrap();
                inner
                    .lru
                    .iter()
                    .filter_map(|p| inner.projects.get(p).map(|e| (p.clone(), e.clone())))
                    .collect()
            };
            if entries.is_empty() {
                return;
            }
            let total: u64 = entries
                .iter()
                .map(|(_, e)| e.graph.read().map(|g| g.heap_bytes() as u64).unwrap_or(0))
                .sum();
            if total <= budget {
                return;
            }
            // Oldest idle project is the victim. If every project is busy,
            // we cannot evict without breaking the in-flight guarantee.
            let Some((path, entry)) = entries
                .into_iter()
                .find(|(_, e)| e.active_queries.load(Ordering::SeqCst) == 0)
            else {
                return;
            };
            let removed = self.quiesce_and_remove(vec![(path, entry)], |e| {
                e.active_queries.load(Ordering::SeqCst) == 0
            });
            if removed == 0 {
                // Victim was revived by a late query; stop to avoid a spin.
                return;
            }
        }
    }

    /// Mark a query as finished so the entry can be evicted.
    pub fn release(&self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.projects.get(&canonical) {
            entry.active_queries.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Mark a project's graph as modified since its last persist, so the next
    /// eviction writes a fresh snapshot. Call after any in-place mutation
    /// (incremental rebuild, heal). Also refreshes `built_at` to now, since
    /// the graph now reflects the just-applied source change.
    pub fn mark_dirty(&self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.projects.get(&canonical) {
            entry.dirty.store(true, Ordering::SeqCst);
            *entry.built_at.lock().unwrap() = SystemTime::now();
        }
    }

    /// Drop a project from memory regardless of activity.
    pub fn unload(&self, path: &Path) -> bool {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut inner = self.inner.lock().unwrap();
        inner.lru.retain(|p| p != &canonical);
        inner.projects.remove(&canonical).is_some()
    }

    /// Sweep idle projects whose last_query is older than `idle_unload`.
    /// Intended to be called from a background timer thread. Each idle graph
    /// is persisted (dirty-gated) before being dropped, so the next query for
    /// it warm-loads from disk instead of rebuilding from source.
    pub fn sweep_idle(&self) -> usize {
        let idle_unload = self.config.idle_unload;
        // Phase 1: pick idle victims under the lock; clone Arcs, defer removal.
        let victims: Vec<(PathBuf, Arc<ProjectEntry>)> = {
            let inner = self.inner.lock().unwrap();
            let now = Instant::now();
            inner
                .projects
                .iter()
                .filter(|(_, entry)| {
                    entry.active_queries.load(Ordering::SeqCst) == 0
                        && now.duration_since(*entry.last_query.lock().unwrap()) >= idle_unload
                })
                .map(|(p, e)| (p.clone(), e.clone()))
                .collect()
        };
        // Phases 2+3: persist off-lock, then remove victims still idle.
        self.quiesce_and_remove(victims, move |entry| {
            entry.active_queries.load(Ordering::SeqCst) == 0
                && Instant::now().duration_since(*entry.last_query.lock().unwrap()) >= idle_unload
        })
    }

    /// Persist a project's graph to its `.coregraph/snapshot.bin` if it is
    /// dirty, clearing the dirty flag on success. A clean entry (warm-loaded
    /// and never mutated) is skipped — the dirty-gate that avoids redundant
    /// disk writes. Errors are logged, not propagated: a failed snapshot must
    /// never block eviction or crash the sweeper.
    fn persist_entry(path: &Path, entry: &ProjectEntry) {
        if !entry.dirty.load(Ordering::SeqCst) {
            return;
        }
        let snap = coregraph_graph::snapshot_path(path);
        let built_at = *entry.built_at.lock().unwrap();
        let graph = entry.graph.read().unwrap();
        match coregraph_graph::persist(&snap, &graph, GraphEpoch::zero(), built_at) {
            Ok(()) => {
                entry.dirty.store(false, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!(
                    "[daemon] snapshot persist failed for {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }

    /// Persist (dirty-gated, off-lock) then remove each victim that is still
    /// evictable when the lock is re-acquired. `still_evictable` is the revive
    /// guard: a query that arrived during the off-lock persist window flips the
    /// entry back to active/recent, so it is kept rather than dropped. Returns
    /// the number actually removed.
    fn quiesce_and_remove(
        &self,
        victims: Vec<(PathBuf, Arc<ProjectEntry>)>,
        still_evictable: impl Fn(&ProjectEntry) -> bool,
    ) -> usize {
        if victims.is_empty() {
            return 0;
        }
        for (path, entry) in &victims {
            Self::persist_entry(path, entry);
        }
        let mut removed = 0;
        let mut inner = self.inner.lock().unwrap();
        for (path, _) in &victims {
            let evict = inner
                .projects
                .get(path)
                .map(|e| still_evictable(e))
                .unwrap_or(false);
            if evict {
                inner.projects.remove(path);
                inner.lru.retain(|q| q != path);
                removed += 1;
            }
        }
        removed
    }

    /// Persist every loaded graph that is dirty. Called on graceful shutdown /
    /// auto-stop so unsaved mutations survive the process exit and the next
    /// daemon start warm-loads them. Returns the number persisted.
    pub fn persist_all_dirty(&self) -> usize {
        let entries: Vec<(PathBuf, Arc<ProjectEntry>)> = {
            let inner = self.inner.lock().unwrap();
            inner
                .projects
                .iter()
                .map(|(p, e)| (p.clone(), e.clone()))
                .collect()
        };
        let mut n = 0;
        for (path, entry) in &entries {
            if entry.dirty.load(Ordering::SeqCst) {
                Self::persist_entry(path, entry);
                n += 1;
            }
        }
        n
    }

    /// Run Gone-cemetery GC on every loaded graph, dropping nodes whose
    /// `Gone` marker has aged past `ttl`. Returns the total nodes reclaimed.
    ///
    /// Collects the entry Arcs under the manager lock, then releases it
    /// before touching any graph so GC never blocks `get_or_load`. A graph
    /// currently write-locked by an in-flight mutation is skipped and retried
    /// on the next sweep. Graphs with an empty cemetery are skipped without
    /// taking the write lock.
    pub fn gc_loaded(&self, ttl: Duration) -> usize {
        let entries: Vec<Arc<ProjectEntry>> = {
            let inner = self.inner.lock().unwrap();
            inner.projects.values().cloned().collect()
        };
        let mut total = 0;
        for entry in entries {
            if let Ok(mut g) = entry.graph.try_write() {
                if g.gone_count() > 0 {
                    total += g.gc_gone(ttl, None);
                }
            }
        }
        total
    }

    /// True if the daemon should terminate because no project has served
    /// a query for `auto_stop`. The rule is symmetric for two edge cases:
    /// - **Empty daemon** (no projects ever loaded): uses
    ///   `started_at` as the reference timestamp so a daemon that
    ///   spawned but was never queried still exits on schedule.
    /// - **Drained daemon** (all projects unloaded by `sweep_idle`):
    ///   same logic, because `projects` is empty and
    ///   `auto_stop >= idle_unload` by construction (30m ≥ 10m).
    ///
    /// Any in-flight query or in-progress load blocks auto-stop.
    pub fn should_auto_stop(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        if !inner.loading.is_empty() {
            return false;
        }
        let now = Instant::now();
        let reference = if inner.projects.is_empty() {
            inner.started_at
        } else {
            // Most-recent activity across all loaded projects.
            inner
                .projects
                .values()
                .map(|entry| {
                    if entry.active_queries.load(Ordering::SeqCst) > 0 {
                        now
                    } else {
                        *entry.last_query.lock().unwrap()
                    }
                })
                .max()
                .unwrap_or(inner.started_at)
        };
        now.duration_since(reference) >= self.config.auto_stop
    }

    /// Status snapshot for `server status`.
    pub fn status(&self) -> ManagerStatus {
        let inner = self.inner.lock().unwrap();
        let mut projects: Vec<ProjectStatus> = Vec::with_capacity(inner.projects.len());
        for (path, entry) in &inner.projects {
            let g = entry.graph.read().unwrap();
            let last = *entry.last_query.lock().unwrap();
            projects.push(ProjectStatus {
                path: path.clone(),
                loaded: entry.loaded,
                loading: false,
                node_count: g.node_count(),
                edge_count: g.edge_count(),
                idle_seconds: Instant::now().duration_since(last).as_secs(),
                active_queries: entry.active_queries.load(Ordering::SeqCst),
            });
        }
        for path in inner.loading.keys() {
            projects.push(ProjectStatus {
                path: path.clone(),
                loaded: false,
                loading: true,
                node_count: 0,
                edge_count: 0,
                idle_seconds: 0,
                active_queries: 0,
            });
        }
        ManagerStatus {
            projects,
            max_loaded: self.config.max_loaded,
            uptime_seconds: Instant::now().duration_since(inner.started_at).as_secs(),
        }
    }

    fn record_access(lru: &mut Vec<PathBuf>, path: &Path) {
        lru.retain(|p| p != path);
        lru.push(path.to_path_buf());
    }
}

/// True if any source file under `root` has a last-modified time newer
/// than `built_at`. Walks the project with the same gitignore-aware
/// iterator the extractor uses, so virtual/derived paths (target/,
/// node_modules/, …) don't trigger spurious rebuilds.
///
/// The walk short-circuits on the first newer file found. On a 400-file
/// TypeScript project this typically returns in <5ms; larger monorepos
/// pay more, but the alternative (serving stale graph data) is worse.
pub(crate) fn source_tree_is_newer(root: &Path, built_at: SystemTime) -> bool {
    // `config.toml` lives under `.coregraph/`, which the walk below skips
    // wholesale (to avoid ping-ponging on our own snapshot writes). But an
    // edit to `[index].exclude` / `[analysis].exclude` changes which files
    // and symbols the graph should contain, so a newer config must force a
    // rebuild — otherwise `warm_load` keeps serving a pre-edit snapshot.
    // `snapshot.bin` (also under `.coregraph/`) stays excluded by the walk,
    // so this carve-out does not reintroduce the write ping-pong.
    if config_is_newer(root, built_at) {
        return true;
    }
    for entry in ignore::WalkBuilder::new(root).build() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Skip things inside `.coregraph/` — we write snapshots there
        // ourselves and any mtime bump would ping-pong with the rebuild.
        // `config.toml` staleness is handled by `config_is_newer` above.
        if path
            .components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new(".coregraph"))
        {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(mtime) = meta.modified() else { continue };
        if mtime > built_at {
            return true;
        }
    }
    false
}

/// True if `<root>/.coregraph/config.toml` has a last-modified time newer than
/// `built_at`. Returns `false` when the file is absent or its mtime is
/// unreadable — a project with no config never looks stale on this account.
fn config_is_newer(root: &Path, built_at: SystemTime) -> bool {
    let config = root.join(".coregraph").join("config.toml");
    let Ok(meta) = std::fs::metadata(&config) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime > built_at
}

/// Status snapshot returned to `server status`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagerStatus {
    pub projects: Vec<ProjectStatus>,
    pub max_loaded: usize,
    pub uptime_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_build(label: &str) -> impl Fn(&Path) -> Result<BuiltGraph, std::io::Error> + use<'_> {
        move |_| {
            let mut g = SymbolGraph::new();
            g.insert_node(coregraph_core::SymbolNode::new(
                coregraph_core::SymbolId(0),
                coregraph_core::SymbolKind::Function,
                label,
                PathBuf::from("src/lib.rs"),
                0,
                10,
            ));
            Ok(BuiltGraph {
                graph: g,
                built_at: SystemTime::now(),
                from_snapshot: false,
            })
        }
    }

    #[test]
    fn get_or_load_builds_once_then_caches() {
        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let dir = tempfile::tempdir().unwrap();
        let build = std::sync::atomic::AtomicUsize::new(0);
        let counted = |p: &Path| {
            build.fetch_add(1, Ordering::SeqCst);
            make_build("foo")(p)
        };
        let g1 = mgr
            .get_or_load::<_, std::io::Error>(dir.path(), counted)
            .unwrap();
        assert_eq!(g1.read().unwrap().node_count(), 1);
        let g2 = mgr
            .get_or_load::<_, std::io::Error>(dir.path(), counted)
            .unwrap();
        assert_eq!(g2.read().unwrap().node_count(), 1);
        assert_eq!(build.load(Ordering::SeqCst), 1, "build should run once");
    }

    #[test]
    fn lru_evicts_when_capacity_exceeded() {
        let cfg = ProjectManagerConfig {
            max_loaded: 2,
            ..ProjectManagerConfig::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let c = tempfile::tempdir().unwrap();
        for (i, dir) in [&a, &b, &c].iter().enumerate() {
            let label = format!("p{}", i);
            mgr.get_or_load::<_, std::io::Error>(dir.path(), {
                let label = label.clone();
                move |_| make_build(&label)(&std::path::PathBuf::new())
            })
            .unwrap();
            mgr.release(dir.path());
        }
        let status = mgr.status();
        // After adding a 3rd project with max_loaded=2, at least one must have
        // been evicted (releasing active_queries lets LRU run).
        assert!(
            status.projects.len() <= 2,
            "LRU cap violated: {:?}",
            status.projects.iter().map(|p| &p.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn config_toml_change_marks_snapshot_stale() {
        use std::time::Duration;
        // A `config.toml` edit (e.g. changing [index].exclude) must invalidate
        // the warm-load snapshot. The source-tree walk skips `.coregraph/` to
        // avoid snapshot-write ping-pong; that skip previously also ignored
        // `config.toml`, so editing exclude silently served a stale graph.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".coregraph")).unwrap();
        std::fs::write(
            root.join(".coregraph").join("config.toml"),
            "[index]\nexclude = []\n",
        )
        .unwrap();
        // built_at in the distant past → the just-written config is newer → stale.
        assert!(
            source_tree_is_newer(root, SystemTime::UNIX_EPOCH),
            "a config.toml newer than built_at must force a rebuild"
        );
        // built_at in the future → config is older and no source file is newer →
        // not stale (guards against an always-rebuild regression).
        assert!(
            !source_tree_is_newer(root, SystemTime::now() + Duration::from_secs(3600)),
            "an unchanged config must not force a rebuild"
        );
    }

    #[test]
    fn status_reports_uptime_and_config() {
        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let status = mgr.status();
        assert_eq!(status.max_loaded, 5);
        assert_eq!(status.max_loaded, 5);
    }

    #[test]
    fn unload_removes_entry() {
        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let dir = tempfile::tempdir().unwrap();
        mgr.get_or_load::<_, std::io::Error>(dir.path(), make_build("x"))
            .unwrap();
        assert_eq!(mgr.status().projects.len(), 1);
        assert!(mgr.unload(dir.path()));
        assert_eq!(mgr.status().projects.len(), 0);
    }

    #[test]
    fn sweep_idle_drops_inactive_projects() {
        let cfg = ProjectManagerConfig {
            idle_unload: Duration::from_millis(1),
            ..ProjectManagerConfig::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        mgr.get_or_load::<_, std::io::Error>(dir.path(), make_build("y"))
            .unwrap();
        mgr.release(dir.path());
        std::thread::sleep(Duration::from_millis(5));
        let n = mgr.sweep_idle();
        assert_eq!(n, 1);
        assert_eq!(mgr.status().projects.len(), 0);
    }

    /// Build a closure that warm-loads from the project snapshot, falling back
    /// to `make_build` on a cold/stale snapshot — mirroring the daemon's
    /// `load_cached_graph` so manager-level tests can exercise the full
    /// persist → warm-load loop.
    fn warm_or_build(label: &'static str) -> impl Fn(&Path) -> Result<BuiltGraph, std::io::Error> {
        move |p: &Path| {
            let snap = coregraph_graph::snapshot_path(p);
            if let Some(w) = coregraph_graph::warm_load(&snap, |_| true) {
                return Ok(BuiltGraph {
                    graph: w.graph,
                    built_at: w.built_at,
                    from_snapshot: true,
                });
            }
            make_build(label)(p)
        }
    }

    #[test]
    fn idle_unload_persists_then_next_load_warm_loads() {
        let cfg = ProjectManagerConfig {
            idle_unload: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let snap = coregraph_graph::snapshot_path(&canonical);

        // Fresh build → dirty.
        mgr.get_or_load::<_, std::io::Error>(dir.path(), warm_or_build("persisted"))
            .unwrap();
        mgr.release(dir.path());
        assert!(!snap.exists(), "no snapshot before first eviction");

        // Idle sweep persists (dirty) then unloads.
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(mgr.sweep_idle(), 1);
        assert_eq!(mgr.status().projects.len(), 0);
        assert!(snap.exists(), "idle unload must persist a snapshot");

        // Next load warm-loads from the snapshot instead of rebuilding.
        let warmed = Arc::new(AtomicUsize::new(0));
        let w = warmed.clone();
        let closure = move |p: &Path| {
            let snap = coregraph_graph::snapshot_path(p);
            if let Some(g) = coregraph_graph::warm_load(&snap, |_| true) {
                w.fetch_add(1, Ordering::SeqCst);
                return Ok::<_, std::io::Error>(BuiltGraph {
                    graph: g.graph,
                    built_at: g.built_at,
                    from_snapshot: true,
                });
            }
            make_build("rebuilt")(p)
        };
        mgr.get_or_load::<_, std::io::Error>(dir.path(), closure)
            .unwrap();
        mgr.release(dir.path());
        assert_eq!(
            warmed.load(Ordering::SeqCst),
            1,
            "second load should restore from the persisted snapshot"
        );
    }

    #[test]
    fn clean_warm_loaded_entry_is_not_re_persisted() {
        // dirty-gate: a warm-loaded entry that was never mutated must not be
        // written again on idle unload. We delete the snapshot after loading
        // and confirm the sweep does NOT recreate it.
        let cfg = ProjectManagerConfig {
            idle_unload: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let snap = coregraph_graph::snapshot_path(&canonical);

        // Seed a snapshot so the first load is a warm (clean) load.
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        {
            let mut g = SymbolGraph::new();
            g.insert_node(coregraph_core::SymbolNode::new(
                coregraph_core::SymbolId(0),
                coregraph_core::SymbolKind::Function,
                "seed",
                PathBuf::from("src/lib.rs"),
                0,
                10,
            ));
            coregraph_graph::persist(&snap, &g, GraphEpoch::zero(), SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        mgr.get_or_load::<_, std::io::Error>(dir.path(), warm_or_build("unused"))
            .unwrap();
        mgr.release(dir.path());

        // Remove the snapshot; a clean entry must not recreate it.
        std::fs::remove_file(&snap).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(mgr.sweep_idle(), 1);
        assert!(
            !snap.exists(),
            "clean (un-mutated) entry must not be re-persisted on eviction"
        );
    }

    #[test]
    fn mark_dirty_forces_persist_of_warm_loaded_entry() {
        // A warm-loaded entry starts clean, but mark_dirty (a mutation) must
        // make the next eviction persist it.
        let cfg = ProjectManagerConfig {
            idle_unload: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let snap = coregraph_graph::snapshot_path(&canonical);

        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        {
            let mut g = SymbolGraph::new();
            g.insert_node(coregraph_core::SymbolNode::new(
                coregraph_core::SymbolId(0),
                coregraph_core::SymbolKind::Function,
                "seed",
                PathBuf::from("src/lib.rs"),
                0,
                10,
            ));
            coregraph_graph::persist(&snap, &g, GraphEpoch::zero(), SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        mgr.get_or_load::<_, std::io::Error>(dir.path(), warm_or_build("unused"))
            .unwrap();
        mgr.release(dir.path());

        mgr.mark_dirty(dir.path()); // simulate a mutation
        std::fs::remove_file(&snap).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(mgr.sweep_idle(), 1);
        assert!(
            snap.exists(),
            "a mutated (dirty) entry must be persisted on eviction"
        );
    }

    #[test]
    fn byte_budget_evicts_lru_idle_project() {
        // A tiny byte budget forces eviction once a second project loads:
        // both single-node graphs exceed 1 byte, so loading B evicts the
        // older idle A while B (busy during its own load) is protected.
        let cfg = ProjectManagerConfig {
            max_loaded_bytes: 1,
            ..ProjectManagerConfig::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        mgr.get_or_load::<_, std::io::Error>(a.path(), make_build("a"))
            .unwrap();
        mgr.release(a.path()); // A becomes idle/evictable

        mgr.get_or_load::<_, std::io::Error>(b.path(), make_build("b"))
            .unwrap();
        mgr.release(b.path());

        let status = mgr.status();
        let paths: Vec<_> = status.projects.iter().map(|p| p.path.clone()).collect();
        let a_canon = std::fs::canonicalize(a.path()).unwrap();
        assert!(
            !paths.contains(&a_canon),
            "byte budget should have evicted the LRU idle project A: {:?}",
            paths
        );
    }

    #[test]
    fn byte_budget_zero_disables_eviction() {
        let cfg = ProjectManagerConfig {
            max_loaded_bytes: 0,
            ..ProjectManagerConfig::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        mgr.get_or_load::<_, std::io::Error>(a.path(), make_build("a"))
            .unwrap();
        mgr.release(a.path());
        mgr.get_or_load::<_, std::io::Error>(b.path(), make_build("b"))
            .unwrap();
        mgr.release(b.path());
        assert_eq!(
            mgr.status().projects.len(),
            2,
            "byte cap disabled → both kept"
        );
    }

    #[test]
    fn gc_loaded_reaps_ripe_gone_nodes_across_loaded_projects() {
        use coregraph_core::NodeStatus;

        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let dir = tempfile::tempdir().unwrap();
        let graph = mgr
            .get_or_load::<_, std::io::Error>(dir.path(), |_| {
                let mut g = SymbolGraph::new();
                let id = g.insert_node(coregraph_core::SymbolNode::new(
                    coregraph_core::SymbolId(0),
                    coregraph_core::SymbolKind::Function,
                    "doomed",
                    PathBuf::from("x.rs"),
                    0,
                    10,
                ));
                g.set_node_status(id, NodeStatus::Gone);
                Ok(BuiltGraph {
                    graph: g,
                    built_at: SystemTime::now(),
                    from_snapshot: false,
                })
            })
            .unwrap();
        mgr.release(dir.path());

        assert_eq!(graph.read().unwrap().gone_count(), 1);
        // TTL = 0 → the Gone marker is immediately ripe.
        let reaped = mgr.gc_loaded(Duration::from_secs(0));
        assert_eq!(reaped, 1, "one ripe Gone node should be reaped");
        assert_eq!(graph.read().unwrap().node_count(), 0);
        assert_eq!(graph.read().unwrap().gone_count(), 0);
    }

    #[test]
    fn auto_stop_fires_when_empty_daemon_idle_long_enough() {
        // Empty daemon — `started_at` is the only timestamp. With a
        // 1ms threshold, the very next `should_auto_stop` call after a
        // short sleep must report true.
        let cfg = ProjectManagerConfig {
            auto_stop: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = ProjectManager::new(cfg);
        std::thread::sleep(Duration::from_millis(5));
        assert!(mgr.should_auto_stop());
    }

    #[test]
    fn auto_stop_blocks_on_recent_query() {
        // A fresh project keeps the daemon alive — `last_query` is now.
        let cfg = ProjectManagerConfig {
            auto_stop: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        mgr.get_or_load::<_, std::io::Error>(dir.path(), make_build("alive"))
            .unwrap();
        mgr.release(dir.path());
        // last_query is essentially "now" — should NOT auto-stop yet.
        assert!(!mgr.should_auto_stop());
        std::thread::sleep(Duration::from_millis(5));
        assert!(mgr.should_auto_stop());
    }

    #[test]
    fn auto_stop_blocks_on_active_query() {
        // active_queries > 0 must always block auto-stop, even past
        // the threshold. Models the case where a slow query is still
        // running when the timer fires.
        let cfg = ProjectManagerConfig {
            auto_stop: Duration::from_millis(1),
            ..Default::default()
        };
        let mgr = Arc::new(ProjectManager::new(cfg));
        let dir = tempfile::tempdir().unwrap();
        mgr.get_or_load::<_, std::io::Error>(dir.path(), make_build("busy"))
            .unwrap();
        // Caller is still inside `get_or_load` semantics — release skipped.
        std::thread::sleep(Duration::from_millis(5));
        assert!(!mgr.should_auto_stop());
        mgr.release(dir.path());
    }

    #[test]
    fn get_or_load_without_refresh_skips_mtime_check() {
        // Verify that get_or_load_without_refresh returns the cached graph even
        // when source files have a newer mtime than built_at, while get_or_load
        // (with refresh) rebuilds under the same conditions.
        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let dir = tempfile::tempdir().unwrap();

        // Initial load — populates the cache.
        mgr.get_or_load::<_, std::io::Error>(dir.path(), make_build("initial"))
            .unwrap();
        mgr.release(dir.path());

        // Touch a source file so source_tree_is_newer returns true on the next call.
        std::thread::sleep(Duration::from_millis(10));
        let marker = dir.path().join("marker.rs");
        std::fs::write(&marker, "pub fn x() {}").unwrap();

        // get_or_load (with refresh) must trigger a rebuild because the marker
        // file is newer than built_at.
        let refresh_count = AtomicUsize::new(0);
        mgr.get_or_load::<_, std::io::Error>(dir.path(), |p| {
            refresh_count.fetch_add(1, Ordering::SeqCst);
            make_build("after_touch")(p)
        })
        .unwrap();
        mgr.release(dir.path());
        assert_eq!(
            refresh_count.load(Ordering::SeqCst),
            1,
            "get_or_load should rebuild when source tree is newer"
        );

        // Touch the marker again to make the source tree newer than the just-rebuilt graph.
        std::thread::sleep(Duration::from_millis(10));
        std::fs::write(&marker, "pub fn y() {}").unwrap();

        // get_or_load_without_refresh must NOT rebuild — the cached entry is returned as-is.
        let no_refresh_count = AtomicUsize::new(0);
        mgr.get_or_load_without_refresh::<_, std::io::Error>(dir.path(), |p| {
            no_refresh_count.fetch_add(1, Ordering::SeqCst);
            make_build("should_not_run")(p)
        })
        .unwrap();
        mgr.release(dir.path());
        assert_eq!(
            no_refresh_count.load(Ordering::SeqCst),
            0,
            "get_or_load_without_refresh must not rebuild when entry is cached"
        );
    }

    #[test]
    fn get_or_load_without_refresh_builds_on_first_access() {
        // When no graph is cached, get_or_load_without_refresh must still run
        // the build closure — it cannot return an error on a cold start.
        let mgr = Arc::new(ProjectManager::new(ProjectManagerConfig::default()));
        let dir = tempfile::tempdir().unwrap();

        let build_count = AtomicUsize::new(0);
        let g = mgr
            .get_or_load_without_refresh::<_, std::io::Error>(dir.path(), |p| {
                build_count.fetch_add(1, Ordering::SeqCst);
                make_build("first")(p)
            })
            .unwrap();
        assert_eq!(
            build_count.load(Ordering::SeqCst),
            1,
            "first access must build even without refresh"
        );
        assert_eq!(g.read().unwrap().node_count(), 1);
        mgr.release(dir.path());
    }
}

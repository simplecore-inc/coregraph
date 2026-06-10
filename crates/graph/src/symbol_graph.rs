use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use coregraph_core::file_state::NodeStatus;
use coregraph_core::{DirectEdge, SymbolId, SymbolNode};
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;

use crate::bloom::SymbolBloom;
use crate::change_tracking::{edge_kind_discriminant, EvidenceIndex, GoneCemetery, GONE_GC_TTL};

/// Fast-exit Levenshtein — bails out as soon as the best-case row distance
/// exceeds `cutoff`, returning `cutoff + 1`. Used for approximate name
/// matching where we only care about candidates within a small threshold.
fn levenshtein_bounded(a: &str, b: &str, cutoff: usize) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let n = a.len();
    let m = b.len();
    if n.abs_diff(m) > cutoff {
        return cutoff + 1;
    }
    if n == 0 {
        return m.min(cutoff + 1);
    }
    if m == 0 {
        return n.min(cutoff + 1);
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        let mut row_min = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = std::cmp::min(
                std::cmp::min(prev[j] + 1, curr[j - 1] + 1),
                prev[j - 1] + cost,
            );
            if curr[j] < row_min {
                row_min = curr[j];
            }
        }
        if row_min > cutoff {
            return cutoff + 1;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// Deduplicates `Arc<Path>` file paths within a graph. The first time a path
/// is seen it is stored; every later identical path returns a clone of the
/// stored `Arc`, so all nodes/edges in one file share a single allocation.
#[derive(Clone, Default)]
struct PathInterner {
    paths: std::collections::HashSet<Arc<Path>>,
}

impl PathInterner {
    /// Return the canonical interned `Arc<Path>` for `path`, inserting it on
    /// first sight. The returned `Arc` shares its allocation with every other
    /// caller that interned an equal path.
    fn intern(&mut self, path: Arc<Path>) -> Arc<Path> {
        if let Some(existing) = self.paths.get(&path) {
            existing.clone()
        } else {
            self.paths.insert(path.clone());
            path
        }
    }

    /// Total bytes of the unique path strings held (counted once each).
    fn total_bytes(&self) -> usize {
        self.paths.iter().map(|p| p.as_os_str().len()).sum()
    }
}

/// petgraph-backed symbol graph.
///
/// Nodes are `SymbolNode`, edges are `DirectEdge`. A `HashMap<SymbolId, NodeIndex>`
/// provides O(1) lookup from domain ids to petgraph indices.
#[derive(Clone)]
pub struct SymbolGraph {
    graph: StableGraph<SymbolNode, DirectEdge>,
    id_map: HashMap<SymbolId, NodeIndex>,
    next_id: u64,
    /// File → nodes / edges that this file provides evidence for. Maintained
    /// incrementally as nodes/edges are inserted. Used by invalidation to
    /// scope "which nodes lived in file X".
    evidence_index: EvidenceIndex,
    /// Graveyard of nodes marked `NodeStatus::Gone`. Entries mature after
    /// `GONE_GC_TTL` and are removed on the next `gc_gone` pass.
    gone: GoneCemetery,
    /// Name → ids. Populated incrementally on insert_node for O(1)
    /// lookup-by-name without scanning every node.
    name_index: HashMap<String, Vec<SymbolId>>,
    /// Qualified name → ids. Populated incrementally. Enables O(1)
    /// lookup-by-fully-qualified-name.
    qualified_index: HashMap<String, Vec<SymbolId>>,
    /// Value → ids. For StringLiteral / EnumVariant / Constant nodes.
    value_index: HashMap<String, Vec<SymbolId>>,
    /// Per-file bloom filter: "does this file define a symbol named X?"
    /// Cheap membership test before the expensive
    /// `graph.nodes().filter(by_file).filter(by_name)` scan. Keyed by the
    /// interned `Arc<Path>` so it shares allocations with the nodes/edges.
    file_blooms: HashMap<Arc<Path>, SymbolBloom>,
    /// Interns `Arc<Path>` file paths so every node/edge in the same file
    /// shares one allocation instead of holding a private copy.
    path_interner: PathInterner,
    /// Seen `(from, to, kind)` triples so `insert_edge` can reject
    /// duplicates. Multiple extractor passes (tree-sitter query +
    /// stack-graphs + string-match mediators) legitimately
    /// emit the same edge for the same symbol pair; without dedup the
    /// graph's `edge_count` drifts across runs as tree-sitter match
    /// order changes. The set is keyed by the same `(from, to,
    /// discriminant)` tuple `evidence_index` already records.
    edge_set: std::collections::HashSet<(SymbolId, SymbolId, u8)>,
    /// Time of last full rebuild. Runtime-only — not persisted in snapshots.
    last_full_rebuild_at: std::time::Instant,
    /// Per-file time of last fast-path update. Runtime-only.
    last_fast_update_at: HashMap<std::path::PathBuf, std::time::Instant>,
}

impl Default for SymbolGraph {
    fn default() -> Self {
        Self {
            graph: Default::default(),
            id_map: Default::default(),
            next_id: 0,
            evidence_index: Default::default(),
            gone: Default::default(),
            name_index: Default::default(),
            qualified_index: Default::default(),
            value_index: Default::default(),
            file_blooms: Default::default(),
            path_interner: Default::default(),
            edge_set: Default::default(),
            last_full_rebuild_at: std::time::Instant::now(),
            last_fast_update_at: HashMap::new(),
        }
    }
}

impl SymbolGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a node into the graph, assigning it a fresh `SymbolId`.
    /// Returns the assigned id.
    pub fn insert_node(&mut self, mut node: SymbolNode) -> SymbolId {
        let id = SymbolId(self.next_id);
        self.next_id += 1;
        node.id = id;
        // Intern the path so all symbols in this file share one allocation.
        node.file = self.path_interner.intern(node.file.clone());
        self.evidence_index.record_node(&node.file, id);
        self.record_indexes(&node, id);
        let idx = self.graph.add_node(node);
        self.id_map.insert(id, idx);
        id
    }

    fn record_indexes(&mut self, node: &SymbolNode, id: SymbolId) {
        self.name_index
            .entry(node.name.clone())
            .or_default()
            .push(id);
        if !node.qualified_name.is_empty() && node.qualified_name != node.name {
            self.qualified_index
                .entry(node.qualified_name.clone())
                .or_default()
                .push(id);
        }
        use coregraph_core::SymbolKind;
        if matches!(
            node.kind,
            SymbolKind::StringLiteral | SymbolKind::EnumVariant | SymbolKind::Constant
        ) {
            self.value_index
                .entry(node.name.clone())
                .or_default()
                .push(id);
        }
        self.file_blooms
            .entry(node.file.clone())
            .or_default()
            .insert(&node.name);
    }

    /// Probabilistic "does `file` define a symbol named `name`?" check.
    /// `false` is definitive; `true` means the caller should fall through
    /// to `nodes_in_file` for confirmation.
    pub fn file_might_define(&self, file: &std::path::Path, name: &str) -> bool {
        match self.file_blooms.get(file) {
            Some(b) => b.might_contain(name),
            None => false,
        }
    }

    /// Number of files with tracked bloom filters (for introspection).
    pub fn file_bloom_count(&self) -> usize {
        self.file_blooms.len()
    }

    /// Approximate heap bytes held by this graph.
    ///
    /// A relative measure for byte-budgeted eviction and introspection, not an
    /// exact RSS figure: it sums the dominant contributors — node and edge
    /// structs plus their owned strings and paths, the id map, and the
    /// name/qualified/value index buckets. It deliberately ignores small
    /// fixed-size bookkeeping (bloom filters, fast-update timestamps) whose
    /// size is bounded by the file count.
    pub fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        let mut bytes = 0usize;
        for n in self.graph.node_weights() {
            // The 16-byte Arc<Path> field is counted in size_of; the path's
            // backing allocation is shared and counted once via the interner.
            bytes += size_of::<SymbolNode>();
            bytes += n.name.len() + n.qualified_name.len();
        }
        bytes += self.graph.edge_count() * size_of::<DirectEdge>();
        bytes += self.path_interner.total_bytes();
        bytes += self.id_map.len() * (size_of::<SymbolId>() + size_of::<NodeIndex>());
        for (k, v) in &self.name_index {
            bytes += k.len() + size_of::<Vec<SymbolId>>() + v.capacity() * size_of::<SymbolId>();
        }
        for (k, v) in &self.qualified_index {
            bytes += k.len() + size_of::<Vec<SymbolId>>() + v.capacity() * size_of::<SymbolId>();
        }
        for (k, v) in &self.value_index {
            bytes += k.len() + size_of::<Vec<SymbolId>>() + v.capacity() * size_of::<SymbolId>();
        }
        bytes += self.edge_set.len() * size_of::<(SymbolId, SymbolId, u8)>();
        bytes
    }

    /// Insert a directed edge between two existing nodes.
    /// Returns `false` if either endpoint is unknown.
    ///
    /// Idempotent: a second insertion of the same `(from, to, kind)`
    /// tuple is a no-op (returns `true` for "edge present" but adds
    /// nothing). This keeps `edge_count()` stable across runs even
    /// when multiple extractor passes emit the same edge.
    pub fn insert_edge(&mut self, mut edge: DirectEdge) -> bool {
        let from_idx = match self.id_map.get(&edge.from) {
            Some(&idx) => idx,
            None => return false,
        };
        let to_idx = match self.id_map.get(&edge.to) {
            Some(&idx) => idx,
            None => return false,
        };
        let key = (edge.from, edge.to, edge_kind_discriminant(&edge.kind));
        if !self.edge_set.insert(key) {
            return true;
        }
        // Intern the evidence path so edges in the same file share it.
        edge.evidence_file = self.path_interner.intern(edge.evidence_file.clone());
        self.evidence_index.record_edge(&edge.evidence_file, key);
        self.graph.add_edge(from_idx, to_idx, edge);
        true
    }

    /// Look up a node by its `SymbolId`.
    pub fn get_node(&self, id: SymbolId) -> Option<&SymbolNode> {
        self.id_map
            .get(&id)
            .and_then(|&idx| self.graph.node_weight(idx))
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Iterate over all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &SymbolNode> {
        self.graph.node_weights()
    }

    /// Iterate over all edges in the graph.
    pub fn edges(&self) -> impl Iterator<Item = &DirectEdge> {
        self.graph.edge_weights()
    }

    /// Edges incident to `id` in both directions, each paired with the
    /// neighbour's `SymbolId` (the endpoint that is not `id`). O(degree) via
    /// petgraph, replacing full O(E) scans in traversal hot paths (impact /
    /// risk BFS, which were O(V·E) per query). Empty if `id` is unknown.
    pub fn incident_edges(&self, id: SymbolId) -> Vec<(SymbolId, &DirectEdge)> {
        let Some(&idx) = self.id_map.get(&id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for e in self
            .graph
            .edges_directed(idx, petgraph::Direction::Outgoing)
        {
            let edge = e.weight();
            out.push((edge.to, edge));
        }
        for e in self
            .graph
            .edges_directed(idx, petgraph::Direction::Incoming)
        {
            let edge = e.weight();
            out.push((edge.from, edge));
        }
        out
    }

    /// Return all edges whose `evidence_file` is `path`. Used by the
    /// fast-path reindex consumers (`reindex_file_in_place` in
    /// `cli/src/dispatch.rs`) to scope which edges should be removed when
    /// a file is reparsed.
    ///
    /// Cost: O(total edges). If this becomes hot, switch to using
    /// `evidence_index.evidence_for(path).outgoing_edge_keys` to get
    /// a set of keys, then look up each edge by key — O(keys) per call.
    pub fn edges_for_file<'a>(
        &'a self,
        path: &'a std::path::Path,
    ) -> impl Iterator<Item = &'a DirectEdge> + 'a {
        self.edges()
            .filter(move |e| e.evidence_file.as_ref() == path)
    }

    /// Remove all edges that match `predicate`, returning how many were removed.
    /// Used by incremental reindex paths to drop edges whose evidence changed.
    pub fn retain_edges<F>(&mut self, mut predicate: F) -> usize
    where
        F: FnMut(&DirectEdge) -> bool,
    {
        let to_remove: Vec<_> = self
            .graph
            .edge_indices()
            .filter(|&idx| {
                self.graph
                    .edge_weight(idx)
                    .map(|e| !predicate(e))
                    .unwrap_or(false)
            })
            .collect();
        let removed = to_remove.len();
        for idx in to_remove {
            self.graph.remove_edge(idx);
        }
        removed
    }

    /// Find all nodes whose `qualified_name` equals `qn`.
    /// Returns at most `limit` matches for bounded memory cost. O(1) amortized.
    pub fn lookup_by_qualified_name<'a>(
        &'a self,
        qn: &'a str,
        limit: usize,
    ) -> Vec<&'a SymbolNode> {
        match self.qualified_index.get(qn) {
            Some(ids) => ids
                .iter()
                .take(limit)
                .filter_map(|id| self.get_node(*id))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Find all nodes whose `name` equals `local`. O(1) amortized via the
    /// incrementally-maintained `name_index`.
    pub fn lookup_by_name<'a>(&'a self, local: &'a str, limit: usize) -> Vec<&'a SymbolNode> {
        match self.name_index.get(local) {
            Some(ids) => ids
                .iter()
                .take(limit)
                .filter_map(|id| self.get_node(*id))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Fuzzy name search — prefix / substring / edit-distance match via
    /// linear scan over the name_index keys (cheap relative to scanning
    /// all nodes because key count = unique names, not total symbols).
    ///
    /// Strategy (evaluated in order, first non-empty result wins):
    ///   1. Case-insensitive substring match against an index key.
    ///   2. Levenshtein distance ≤ 2 between the query and index key.
    pub fn lookup_by_name_fuzzy<'a>(&'a self, query: &'a str, limit: usize) -> Vec<&'a SymbolNode> {
        let lower = query.to_ascii_lowercase();

        // Stage 1: substring match, ranked. Without ranking a query like
        // "Excalidraw" on a large repo would return a ConfigKey whose name
        // happens to include "excalidraw" (e.g. "services.excalidraw.x")
        // before the actual `Excalidraw` class. Rank: (score, id) where
        // lower is better.
        //   0 = exact case-insensitive match
        //   1 = name starts with query
        //   2 = other substring match, shorter names first (ties broken by
        //       name length to prefer the tightest match)
        let mut substr: Vec<(usize, usize, &SymbolNode)> = Vec::new();
        for (name, ids) in &self.name_index {
            let name_lower = name.to_ascii_lowercase();
            if !name_lower.contains(&lower) {
                continue;
            }
            let score = if name_lower == lower {
                0
            } else if name_lower.starts_with(&lower) {
                1
            } else {
                2
            };
            let length_tiebreak = name.len();
            for &id in ids {
                if let Some(node) = self.get_node(id) {
                    substr.push((score, length_tiebreak, node));
                }
            }
        }
        if !substr.is_empty() {
            substr.sort_by_key(|(score, len, _)| (*score, *len));
            return substr.into_iter().take(limit).map(|(_, _, n)| n).collect();
        }

        // Stage 2: Levenshtein-prefix ≤ 2. We compare the query against the
        // leading `query.len()` bytes of each index key so that short typos
        // like `Hok` still surface `HookRegistry` / `HookEntry` (which
        // match the prefix `Hoo` within distance 1).
        let mut by_edit: Vec<(usize, &SymbolNode)> = Vec::new();
        let max_allowed = if query.len() < 4 { 1 } else { 2 };
        for (name, ids) in &self.name_index {
            let prefix_len = std::cmp::min(name.len(), query.len() + max_allowed);
            let prefix_lower = name[..prefix_len].to_ascii_lowercase();
            let d = levenshtein_bounded(&lower, &prefix_lower, max_allowed + 1);
            if d > max_allowed {
                continue;
            }
            for &id in ids {
                if let Some(node) = self.get_node(id) {
                    by_edit.push((d, node));
                }
            }
        }
        by_edit.sort_by_key(|(d, _)| *d);
        by_edit.into_iter().take(limit).map(|(_, n)| n).collect()
    }

    /// Find all nodes whose value (node.name for StringLiteral / EnumVariant /
    /// Constant) equals `value`. O(1) amortized.
    pub fn lookup_by_value<'a>(&'a self, value: &'a str, limit: usize) -> Vec<&'a SymbolNode> {
        match self.value_index.get(value) {
            Some(ids) => ids
                .iter()
                .take(limit)
                .filter_map(|id| self.get_node(*id))
                .collect(),
            None => Vec::new(),
        }
    }

    /// Find the enclosing definition node in `file` whose span contains `byte_offset`.
    /// When multiple candidates exist (nested definitions), returns the narrowest span.
    pub fn enclosing_node_at<'a>(
        &'a self,
        file: &std::path::Path,
        byte_offset: u32,
    ) -> Option<&'a SymbolNode> {
        let mut best: Option<&SymbolNode> = None;
        for node in self.graph.node_weights() {
            if node.file.as_ref() != file {
                continue;
            }
            if byte_offset < node.span_start || byte_offset >= node.span_end {
                continue;
            }
            match best {
                None => best = Some(node),
                Some(current) => {
                    let current_width = current.span_end - current.span_start;
                    let candidate_width = node.span_end - node.span_start;
                    if candidate_width < current_width {
                        best = Some(node);
                    }
                }
            }
        }
        best
    }

    /// Iterate over all nodes defined in the given file.
    pub fn nodes_in_file<'a>(
        &'a self,
        file: &'a std::path::Path,
    ) -> impl Iterator<Item = &'a SymbolNode> + 'a {
        self.graph
            .node_weights()
            .filter(move |n| n.file.as_ref() == file)
    }

    /// Insert a node while preserving its existing SymbolId (used for snapshot restoration).
    /// Advances next_id if the provided id would otherwise collide with future auto-assigns.
    pub fn insert_node_preserving_id(&mut self, node: SymbolNode) -> SymbolId {
        let id = node.id;
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
        self.evidence_index.record_node(&node.file, id);
        self.record_indexes(&node, id);
        let idx = self.graph.add_node(node);
        self.id_map.insert(id, idx);
        id
    }

    /// Per-file evidence index. Answers "which nodes and edges belong to
    /// this file" in O(1).
    pub fn evidence_index(&self) -> &EvidenceIndex {
        &self.evidence_index
    }

    /// Mark a node's status in-place. Recording `NodeStatus::Gone` also
    /// schedules the id for eventual GC by `gc_gone`.
    pub fn set_node_status(&mut self, id: SymbolId, status: NodeStatus) -> bool {
        let Some(&idx) = self.id_map.get(&id) else {
            return false;
        };
        let Some(node) = self.graph.node_weight_mut(idx) else {
            return false;
        };
        node.status = status;
        if status == NodeStatus::Gone {
            self.gone.mark(id);
        } else {
            self.gone.forget(id);
        }
        true
    }

    /// Mark every node defined in `file` with `status`. Returns the number
    /// of nodes touched.
    pub fn mark_file_nodes(&mut self, file: &std::path::Path, status: NodeStatus) -> usize {
        let Some(evidence) = self.evidence_index.evidence_for(file) else {
            return 0;
        };
        let ids: Vec<_> = evidence.defined_nodes.iter().copied().collect();
        let mut n = 0;
        for id in ids {
            if self.set_node_status(id, status) {
                n += 1;
            }
        }
        n
    }

    /// Number of nodes currently in the `Gone` cemetery awaiting GC.
    pub fn gone_count(&self) -> usize {
        self.gone.len()
    }

    /// Drop every node whose `Gone` marker is older than `ttl`. Uses the
    /// real wall clock unless `now` is provided (for deterministic tests).
    /// Delegates to `remove_node` to ensure all indexes are cleaned up.
    ///
    /// # Return value semantics
    ///
    /// Returns the number of ripe ids scheduled for removal — which, under
    /// the invariant below, equals the number of actual structural removals.
    ///
    /// **Invariant**: every id recorded in `self.gone` is also present in
    /// `self.id_map`. This holds because:
    /// 1. `set_node_status(id, Gone)` only inserts into `self.gone` after
    ///    successfully locating `id` in `self.id_map`.
    /// 2. Every code path that removes an id from `self.id_map` (currently
    ///    only `remove_node`) also calls `self.gone.forget(id)`.
    ///
    /// Therefore a ripe id always has a corresponding node to remove, and
    /// the ripe count equals the actual removal count. If this invariant is
    /// ever violated, the returned value will over-count and callers that
    /// rely on it (e.g. for logging "freed N symbols") will be misleading.
    pub fn gc_gone(&mut self, ttl: Duration, now: Option<SystemTime>) -> usize {
        let now = now.unwrap_or_else(SystemTime::now);
        let ripe = self.gone.ripe(ttl, now);
        let count = ripe.len();
        for id in ripe {
            // remove_node also calls gone.forget internally
            self.remove_node(id);
        }
        count
    }

    /// Run GC with the default 5-minute grace window.
    pub fn gc_gone_default(&mut self) -> usize {
        self.gc_gone(GONE_GC_TTL, None)
    }

    /// Time the graph was last fully rebuilt (via `build_graph` or `mark_full_rebuild`).
    pub fn last_full_rebuild_at(&self) -> std::time::Instant {
        self.last_full_rebuild_at
    }

    /// Time this file was last fast-path updated, if any.
    pub fn last_fast_update_at(&self, path: &std::path::Path) -> Option<std::time::Instant> {
        self.last_fast_update_at.get(path).copied()
    }

    /// Reset freshness — records the current time as the last full rebuild.
    /// Clears per-file fast-update map.
    pub fn mark_full_rebuild(&mut self) {
        self.last_full_rebuild_at = std::time::Instant::now();
        self.last_fast_update_at.clear();
    }

    /// Record a fast-path update for a file.
    pub fn mark_fast_update(&mut self, path: &std::path::Path) {
        self.last_fast_update_at
            .insert(path.to_path_buf(), std::time::Instant::now());
    }

    /// Remove a node and all incident edges. Cleans up name/qualified/value
    /// indexes, edge_set, and evidence_index. `file_blooms` keeps an
    /// over-approximating entry while the file still has live nodes (a
    /// bloom cannot drop a single name); the entry is removed once the
    /// file's last node is gone, so the map stays bounded by live files.
    /// Idempotent: missing id is a no-op.
    pub fn remove_node(&mut self, id: SymbolId) {
        use coregraph_core::SymbolKind;
        let idx = match self.id_map.remove(&id) {
            Some(idx) => idx,
            None => return,
        };
        // Snapshot the fields we need before petgraph drops the weight.
        let (name, qualified_name, kind, file) = {
            let node = match self.graph.node_weight(idx) {
                Some(n) => n,
                None => return,
            };
            (
                node.name.clone(),
                node.qualified_name.clone(),
                node.kind.clone(),
                node.file.clone(),
            )
        };
        // Removes the node AND every incident edge in petgraph.
        self.graph.remove_node(idx);

        // Name index
        if let Some(ids) = self.name_index.get_mut(&name) {
            ids.retain(|x| *x != id);
            if ids.is_empty() {
                self.name_index.remove(&name);
            }
        }
        // Qualified index (only if distinct from name — record_indexes skips equal-to-name)
        if !qualified_name.is_empty() && qualified_name != name {
            if let Some(ids) = self.qualified_index.get_mut(&qualified_name) {
                ids.retain(|x| *x != id);
                if ids.is_empty() {
                    self.qualified_index.remove(&qualified_name);
                }
            }
        }
        // Value index (only for nodes recorded there)
        if matches!(
            kind,
            SymbolKind::StringLiteral | SymbolKind::EnumVariant | SymbolKind::Constant
        ) {
            if let Some(ids) = self.value_index.get_mut(&name) {
                ids.retain(|x| *x != id);
                if ids.is_empty() {
                    self.value_index.remove(&name);
                }
            }
        }
        // edge_set: drop any triple touching this id.
        // Cost: O(total edges) — retain walks the full set. Acceptable for
        // the fast-path invalidation use case (single node at a time) but
        // a batch remove helper should be considered if this becomes hot.
        self.edge_set
            .retain(|(from, to, _)| *from != id && *to != id);
        // evidence_index: drop node entry + any edge keys that touched this id
        self.evidence_index.remove_node(id);
        self.evidence_index.remove_edges_incident_to(id);
        // Also evict from the gone cemetery if present (e.g. forced removal during GC)
        self.gone.forget(id);
        // file_blooms: a bloom filter cannot drop a single name, but once a
        // file has no live nodes its whole entry is pure leak — drop it then.
        // An over-approximating bloom for a still-populated file only costs an
        // extra confirmation scan in `file_might_define`, so those are left.
        let file_still_populated = self.graph.node_weights().any(|n| n.file == file);
        if !file_still_populated {
            self.file_blooms.remove(&file);
        }
    }

    /// Increment `stale_evidence_count` on the specific `(from, to, kind)`
    /// edge. No-op if the edge does not exist. Used by fast-path reindex
    /// to mark cross-file edges as potentially stale without a full graph
    /// rebuild.
    pub fn bump_stale_evidence(
        &mut self,
        from: SymbolId,
        to: SymbolId,
        kind: coregraph_core::EdgeKind,
    ) {
        let Some(&f_idx) = self.id_map.get(&from) else {
            return;
        };
        let Some(&t_idx) = self.id_map.get(&to) else {
            return;
        };
        let target_disc = edge_kind_discriminant(&kind);
        // edges_connecting is O(degree) — preferred over full edge_indices scan.
        let eidx = self
            .graph
            .edges_connecting(f_idx, t_idx)
            .find(|e| edge_kind_discriminant(&e.weight().kind) == target_disc)
            .map(|e| e.id());
        if let Some(eidx) = eidx {
            if let Some(w) = self.graph.edge_weight_mut(eidx) {
                w.stale_evidence_count = w.stale_evidence_count.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::{
        edge::{AnalysisOrigin, Confidence},
        EdgeKind, SymbolKind,
    };
    use std::path::PathBuf;

    fn make_node(name: &str, kind: SymbolKind) -> SymbolNode {
        SymbolNode::new(SymbolId(0), kind, name, PathBuf::from("src/lib.rs"), 0, 10)
    }

    #[test]
    fn insert_and_retrieve_nodes() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("foo", SymbolKind::Function));
        let id_b = g.insert_node(make_node("Bar", SymbolKind::Class));

        assert_ne!(id_a, id_b);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.get_node(id_a).unwrap().name, "foo");
        assert_eq!(g.get_node(id_b).unwrap().name, "Bar");
    }

    #[test]
    fn insert_edge() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("caller", SymbolKind::Function));
        let id_b = g.insert_node(make_node("callee", SymbolKind::Function));

        let edge = DirectEdge::new(
            id_a,
            id_b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("src/lib.rs"),
        );
        assert!(g.insert_edge(edge));
        assert_eq!(g.edge_count(), 1);
    }

    #[test]
    fn unknown_id_returns_none() {
        let g = SymbolGraph::new();
        assert!(g.get_node(SymbolId(999)).is_none());
    }

    #[test]
    fn nodes_iterator() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node("A", SymbolKind::Function));
        g.insert_node(make_node("B", SymbolKind::Class));

        let names: Vec<&str> = g.nodes().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn insert_edge_unknown_endpoint_returns_false() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("A", SymbolKind::Function));

        let edge = DirectEdge::new(
            id_a,
            SymbolId(999),
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.8),
            PathBuf::from("src/lib.rs"),
        );
        assert!(!g.insert_edge(edge));
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn evidence_index_tracks_defined_nodes() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b",
            PathBuf::from("src/b.rs"),
            0,
            10,
        ));
        let evidence = g.evidence_index().evidence_for(&PathBuf::from("src/a.rs"));
        assert!(evidence.is_some());
        assert!(evidence.unwrap().defined_nodes.contains(&id_a));
    }

    #[test]
    fn mark_file_nodes_stale_updates_status() {
        use coregraph_core::file_state::NodeStatus;
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        assert_eq!(g.get_node(id_a).unwrap().status, NodeStatus::Verified);
        let marked = g.mark_file_nodes(&PathBuf::from("src/a.rs"), NodeStatus::Stale);
        assert_eq!(marked, 1);
        assert_eq!(g.get_node(id_a).unwrap().status, NodeStatus::Stale);
    }

    #[test]
    fn gc_gone_removes_ripe_nodes() {
        use coregraph_core::file_state::NodeStatus;
        use std::time::{Duration, SystemTime};
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "doomed",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        g.mark_file_nodes(&PathBuf::from("src/a.rs"), NodeStatus::Gone);
        assert_eq!(g.gone_count(), 1);
        assert_eq!(g.node_count(), 1);
        // Pretend we're looking 10 minutes in the future — past the 5-min TTL.
        let future = SystemTime::now() + Duration::from_secs(600);
        let removed = g.gc_gone(Duration::from_secs(300), Some(future));
        assert_eq!(removed, 1);
        assert_eq!(g.node_count(), 0);
    }

    #[test]
    fn name_index_returns_exact_matches() {
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookRegistry",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookRegistry",
            PathBuf::from("src/b.rs"),
            0,
            10,
        ));
        let hits = g.lookup_by_name("HookRegistry", 10);
        assert_eq!(hits.len(), 2);
        let ids: std::collections::HashSet<_> = hits.iter().map(|n| n.id).collect();
        assert!(ids.contains(&a));
        assert!(ids.contains(&b));
    }

    #[test]
    fn name_index_fuzzy_edit_distance_matches_hok() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookRegistry",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookEntry",
            PathBuf::from("src/b.rs"),
            0,
            10,
        ));
        let hits = g.lookup_by_name_fuzzy("Hok", 10);
        let names: Vec<&str> = hits.iter().map(|n| n.name.as_str()).collect();
        assert!(
            names.contains(&"HookRegistry"),
            "expected HookRegistry via Levenshtein-prefix: {:?}",
            names
        );
    }

    #[test]
    fn name_index_fuzzy_prefix_and_substring() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookRegistry",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "HookEntry",
            PathBuf::from("src/b.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "unrelated",
            PathBuf::from("src/c.rs"),
            0,
            10,
        ));
        let hits = g.lookup_by_name_fuzzy("Hok", 10);
        // Strict substring wouldn't match "Hok" against "Hook*", but the
        // relaxed lowercase-contains match should succeed for this case-
        // insensitive form. The test documents the current tolerance.
        assert!(
            hits.iter().all(|n| n.name != "unrelated"),
            "unrelated names should not appear: {:?}",
            hits.iter().map(|n| &n.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn qualified_index_prefers_qualified_name() {
        let mut g = SymbolGraph::new();
        g.insert_node(
            SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                "bar",
                PathBuf::from("Foo.java"),
                0,
                10,
            )
            .with_qualified_name("com.example.Foo.bar"),
        );
        let by_qn = g.lookup_by_qualified_name("com.example.Foo.bar", 10);
        assert_eq!(by_qn.len(), 1);
    }

    #[test]
    fn file_bloom_reports_negative_definitively() {
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "foo",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        assert!(g.file_might_define(std::path::Path::new("src/a.rs"), "foo"));
        assert!(!g.file_might_define(std::path::Path::new("src/missing.rs"), "foo"));
        // Negative file/name pair: bloom has a small false-positive rate;
        // we only assert that a distinctly different name is negative.
        assert!(!g.file_might_define(std::path::Path::new("src/a.rs"), "zzzzztotally_unique_nope"));
    }

    #[test]
    fn value_index_tracks_string_literals() {
        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::StringLiteral,
            "api_path::/api/v1/cards",
            PathBuf::from("a.java"),
            0,
            10,
        ));
        let by_value = g.lookup_by_value("api_path::/api/v1/cards", 10);
        assert_eq!(by_value.len(), 1);
        assert_eq!(by_value[0].id, a);
    }

    #[test]
    fn name_index_lookup_is_faster_than_linear_scan() {
        use std::time::Instant;
        // Build a graph with 1000 distinct names.
        let mut g = SymbolGraph::new();
        for i in 0..1000 {
            g.insert_node(SymbolNode::new(
                SymbolId(0),
                SymbolKind::Function,
                format!("sym_{}", i),
                PathBuf::from(format!("src/f{}.rs", i % 20)),
                0,
                10,
            ));
        }
        let target = "sym_500";

        // Linear scan baseline.
        let linear_runs = 1000;
        let t0 = Instant::now();
        for _ in 0..linear_runs {
            let _ = g.nodes().filter(|n| n.name == target).collect::<Vec<_>>();
        }
        let linear = t0.elapsed();

        // Index lookup.
        let t1 = Instant::now();
        for _ in 0..linear_runs {
            let _ = g.lookup_by_name(target, 8);
        }
        let indexed = t1.elapsed();

        // Index should be at least 10× faster on this workload.
        let speedup = linear.as_nanos() as f64 / indexed.as_nanos().max(1) as f64;
        eprintln!(
            "[name_index_lookup_is_faster_than_linear_scan] linear={:?}, indexed={:?}, speedup={:.1}x",
            linear, indexed, speedup
        );
        assert!(
            speedup >= 10.0,
            "expected ≥10× speedup, got {:.1}× (linear {:?}, indexed {:?})",
            speedup,
            linear,
            indexed
        );
    }

    #[test]
    fn insert_node_preserving_id_populates_indexes() {
        // Regression: snapshot restoration must re-populate name_index,
        // qualified_index, value_index, and file_blooms. Prior to the fix
        // these were silently left empty, breaking every index lookup
        // after a graph load from disk.
        let mut g = SymbolGraph::new();
        let restored_fn = SymbolNode::new(
            SymbolId(42),
            SymbolKind::Function,
            "restored_fn",
            PathBuf::from("src/restored.rs"),
            0,
            10,
        )
        .with_qualified_name("pkg.module.restored_fn");
        let restored_literal = SymbolNode::new(
            SymbolId(43),
            SymbolKind::StringLiteral,
            "api_path::/api/v1/users",
            PathBuf::from("src/restored.rs"),
            20,
            30,
        );

        let id_fn = g.insert_node_preserving_id(restored_fn);
        let id_lit = g.insert_node_preserving_id(restored_literal);

        assert_eq!(id_fn, SymbolId(42));
        assert_eq!(id_lit, SymbolId(43));

        // name_index
        let by_name = g.lookup_by_name("restored_fn", 8);
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].id, id_fn);

        // qualified_index
        let by_qname = g.lookup_by_qualified_name("pkg.module.restored_fn", 8);
        assert_eq!(by_qname.len(), 1);
        assert_eq!(by_qname[0].id, id_fn);

        // value_index (string literals are indexed by name)
        let by_value = g.lookup_by_value("api_path::/api/v1/users", 8);
        assert_eq!(by_value.len(), 1);
        assert_eq!(by_value[0].id, id_lit);

        // file_blooms
        assert!(g.file_might_define(&PathBuf::from("src/restored.rs"), "restored_fn"));
        assert!(g.file_might_define(&PathBuf::from("src/restored.rs"), "api_path::/api/v1/users"));
    }

    #[test]
    fn gc_gone_spares_young_entries() {
        use coregraph_core::file_state::NodeStatus;
        use std::time::{Duration, SystemTime};
        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "fresh_gone",
            PathBuf::from("src/a.rs"),
            0,
            10,
        ));
        g.mark_file_nodes(&PathBuf::from("src/a.rs"), NodeStatus::Gone);
        // Ask for GC at the current moment — TTL not elapsed.
        let removed = g.gc_gone(Duration::from_secs(300), Some(SystemTime::now()));
        assert_eq!(removed, 0);
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn new_symbol_graph_records_creation_time() {
        let g = SymbolGraph::new();
        assert!(g.last_full_rebuild_at().elapsed().as_secs() < 5);
        assert!(g
            .last_fast_update_at(std::path::Path::new("nonexistent"))
            .is_none());
    }

    #[test]
    fn mark_fast_update_records_per_file_time() {
        let mut g = SymbolGraph::new();
        g.mark_fast_update(std::path::Path::new("foo.rs"));
        assert!(g
            .last_fast_update_at(std::path::Path::new("foo.rs"))
            .is_some());
        assert!(g
            .last_fast_update_at(std::path::Path::new("bar.rs"))
            .is_none());
    }

    #[test]
    fn mark_full_rebuild_clears_fast_update_map() {
        let mut g = SymbolGraph::new();
        g.mark_fast_update(std::path::Path::new("foo.rs"));
        assert!(g
            .last_fast_update_at(std::path::Path::new("foo.rs"))
            .is_some());
        g.mark_full_rebuild();
        assert!(g
            .last_fast_update_at(std::path::Path::new("foo.rs"))
            .is_none());
    }

    #[test]
    fn remove_node_drops_node_and_incident_edges_and_clears_indexes() {
        use coregraph_core::edge::{AnalysisOrigin, Confidence};
        use coregraph_core::{DirectEdge, EdgeKind, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        let foo = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Foo",
            PathBuf::from("a.rs"),
            0,
            3,
        ));
        let bar = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Bar",
            PathBuf::from("a.rs"),
            10,
            13,
        ));
        g.insert_edge(DirectEdge::new(
            foo,
            bar,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        g.insert_edge(DirectEdge::new(
            bar,
            foo,
            EdgeKind::References,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));

        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 2);
        assert!(g.get_node(foo).is_some());
        assert!(!g.lookup_by_name("Foo", 10).is_empty());

        g.remove_node(foo);

        assert!(g.get_node(foo).is_none(), "node itself should be gone");
        assert_eq!(g.node_count(), 1, "only Bar should remain");
        assert_eq!(g.edge_count(), 0, "both incident edges should be gone");
        assert!(
            g.lookup_by_name("Foo", 10).is_empty(),
            "name index must no longer resolve to this id"
        );
        assert!(
            g.edges().find(|e| e.from == foo || e.to == foo).is_none(),
            "no dangling edges"
        );
    }

    #[test]
    fn insert_interns_paths_so_same_file_shares_one_allocation() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        // Two nodes constructed from independent PathBufs of the same file.
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a",
            PathBuf::from("src/same.rs"),
            0,
            10,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b",
            PathBuf::from("src/same.rs"),
            20,
            30,
        ));
        // Clone the Arcs (ptr identity preserved) so we can keep inserting.
        let fa = g.get_node(a).unwrap().file.clone();
        let fb = g.get_node(b).unwrap().file.clone();
        assert!(
            std::sync::Arc::ptr_eq(&fa, &fb),
            "two symbols in the same file must share one interned Arc<Path>"
        );

        // A different file must NOT share the allocation.
        let c = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "c",
            PathBuf::from("src/other.rs"),
            0,
            10,
        ));
        let fc = g.get_node(c).unwrap().file.clone();
        assert!(!std::sync::Arc::ptr_eq(&fa, &fc));
    }

    #[test]
    fn heap_bytes_grows_with_nodes_and_is_nonzero() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let empty = SymbolGraph::new();
        assert_eq!(empty.heap_bytes(), 0, "empty graph holds no node/edge heap");

        let mut g = SymbolGraph::new();
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "short",
            PathBuf::from("a.rs"),
            0,
            10,
        ));
        let one = g.heap_bytes();
        assert!(one > 0);

        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a_much_longer_symbol_name",
            PathBuf::from("some/deeply/nested/path/file.rs"),
            0,
            10,
        ));
        assert!(
            g.heap_bytes() > one,
            "adding a node must increase heap_bytes"
        );
    }

    #[test]
    fn remove_node_drops_file_bloom_when_file_empties() {
        use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        // Two files: a.rs has one node, b.rs has two.
        let a0 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "a_fn",
            PathBuf::from("a.rs"),
            0,
            10,
        ));
        g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b_fn1",
            PathBuf::from("b.rs"),
            0,
            10,
        ));
        let b1 = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "b_fn2",
            PathBuf::from("b.rs"),
            0,
            10,
        ));
        assert_eq!(g.file_bloom_count(), 2);

        // Removing a.rs's only node must drop its bloom entry.
        g.remove_node(a0);
        assert_eq!(g.file_bloom_count(), 1, "a.rs bloom should be gone");
        assert!(!g.file_might_define(&PathBuf::from("a.rs"), "a_fn"));

        // Removing one of b.rs's two nodes must NOT drop b.rs's bloom.
        g.remove_node(b1);
        assert_eq!(g.file_bloom_count(), 1, "b.rs still has a live node");
    }

    #[test]
    fn remove_node_missing_id_is_noop() {
        let mut g = SymbolGraph::new();
        // No nodes. Should not panic.
        g.remove_node(SymbolId(999));
        assert_eq!(g.node_count(), 0);
    }

    #[test]
    fn bump_stale_evidence_increments_matching_edge_only() {
        use coregraph_core::edge::{AnalysisOrigin, Confidence};
        use coregraph_core::{DirectEdge, EdgeKind, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "A",
            PathBuf::from("a.rs"),
            0,
            1,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "B",
            PathBuf::from("b.rs"),
            0,
            1,
        ));

        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::References,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));

        g.bump_stale_evidence(a, b, EdgeKind::Calls);

        let calls = g
            .edges()
            .find(|e| e.from == a && e.to == b && matches!(e.kind, EdgeKind::Calls))
            .unwrap();
        assert_eq!(calls.stale_evidence_count, 1);

        let refs = g
            .edges()
            .find(|e| e.from == a && e.to == b && matches!(e.kind, EdgeKind::References))
            .unwrap();
        assert_eq!(
            refs.stale_evidence_count, 0,
            "different kind on same pair must NOT be bumped"
        );
    }

    #[test]
    fn bump_stale_evidence_unknown_edge_is_noop() {
        let mut g = SymbolGraph::new();
        g.bump_stale_evidence(SymbolId(0), SymbolId(1), coregraph_core::EdgeKind::Calls);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn gc_gone_cleans_all_indexes_not_just_id_map() {
        // Regression: before the gc_gone → remove_node refactor, GC left
        // stale entries in name_index / qualified_index / value_index /
        // edge_set / evidence_index. This test exercises that path.
        use coregraph_core::file_state::NodeStatus;
        use coregraph_core::{SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        let id = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "UniqueNameThatOnlyExistsOnce",
            PathBuf::from("gc-index-test.rs"),
            0,
            5,
        ));

        // Baseline — every index resolves.
        assert!(!g
            .lookup_by_name("UniqueNameThatOnlyExistsOnce", 10)
            .is_empty());
        assert!(g
            .evidence_index()
            .evidence_for(&PathBuf::from("gc-index-test.rs"))
            .is_some());

        // Mark Gone and pretend the TTL has elapsed.
        g.set_node_status(id, NodeStatus::Gone);
        let future = std::time::SystemTime::now() + GONE_GC_TTL + std::time::Duration::from_secs(1);
        let removed = g.gc_gone(GONE_GC_TTL, Some(future));
        assert_eq!(removed, 1);

        // After GC, every index must forget the node.
        assert!(
            g.lookup_by_name("UniqueNameThatOnlyExistsOnce", 10).is_empty(),
            "name_index must no longer resolve to the GC'd id (the pre-existing leak this refactor fixes)"
        );
        assert!(
            g.evidence_index()
                .evidence_for(&PathBuf::from("gc-index-test.rs"))
                .is_none(),
            "evidence_index must drop the file entry when it has no remaining nodes"
        );
        assert!(g.get_node(id).is_none());
    }

    #[test]
    fn remove_node_with_shared_name_spares_other_ids() {
        // Catches a retain-by-value bug: if remove_node cleared the entire
        // name_index bucket instead of filtering by id, the surviving
        // "Shared" node would no longer resolve.
        use coregraph_core::{SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        let victim = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Shared",
            PathBuf::from("a.rs"),
            0,
            5,
        ));
        let survivor = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "Shared",
            PathBuf::from("b.rs"),
            0,
            5,
        ));

        g.remove_node(victim);

        let hits = g.lookup_by_name("Shared", 10);
        assert_eq!(hits.len(), 1, "surviving 'Shared' must still resolve");
        assert_eq!(hits[0].id, survivor);
    }

    #[test]
    fn remove_node_cleans_cross_file_edge_evidence() {
        // A node in a.rs with an outgoing edge evidenced in b.rs must
        // clear the edge key from b.rs's evidence set when the node is
        // removed. Exercises remove_edges_incident_to's cross-file sweep.
        use coregraph_core::edge::{AnalysisOrigin, Confidence};
        use coregraph_core::{DirectEdge, EdgeKind, SymbolKind, SymbolNode};
        use std::path::PathBuf;

        let mut g = SymbolGraph::new();
        let caller = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "caller",
            PathBuf::from("a.rs"),
            0,
            5,
        ));
        let callee = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "callee",
            PathBuf::from("b.rs"),
            0,
            5,
        ));
        // Edge FROM caller (a.rs) TO callee (b.rs) with evidence in b.rs —
        // a legitimate shape when the call site lives alongside the callee
        // (e.g. a reverse-resolved cross-file reference).
        g.insert_edge(DirectEdge::new(
            caller,
            callee,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("b.rs"),
        ));

        // Baseline: b.rs has one outgoing_edge_key.
        let b_evidence = g
            .evidence_index()
            .evidence_for(&PathBuf::from("b.rs"))
            .expect("b.rs must have an evidence set");
        assert_eq!(b_evidence.outgoing_edge_keys.len(), 1);

        g.remove_node(caller);

        // b.rs's evidence set no longer contains the edge key. Since callee
        // still defines a node in b.rs, the file key itself remains — but
        // outgoing_edge_keys must be empty.
        let b_evidence_after = g
            .evidence_index()
            .evidence_for(&PathBuf::from("b.rs"))
            .expect("b.rs still hosts callee, so the file key stays");
        assert!(
            b_evidence_after.outgoing_edge_keys.is_empty(),
            "cross-file edge evidence must be cleaned when its endpoint is removed"
        );
    }

    #[test]
    fn edges_for_file_returns_only_edges_with_matching_evidence() {
        use coregraph_core::edge::{AnalysisOrigin, Confidence};
        use coregraph_core::{DirectEdge, EdgeKind, SymbolKind, SymbolNode};
        use std::path::{Path, PathBuf};

        let mut g = SymbolGraph::new();
        let a = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "A",
            PathBuf::from("a.rs"),
            0,
            1,
        ));
        let b = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "B",
            PathBuf::from("b.rs"),
            0,
            1,
        ));
        let c = g.insert_node(SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            "C",
            PathBuf::from("c.rs"),
            0,
            1,
        ));
        // Two edges evidenced in a.rs, one in b.rs.
        g.insert_edge(DirectEdge::new(
            a,
            b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        g.insert_edge(DirectEdge::new(
            a,
            c,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        ));
        g.insert_edge(DirectEdge::new(
            b,
            c,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("b.rs"),
        ));

        let a_edges: Vec<_> = g.edges_for_file(Path::new("a.rs")).collect();
        assert_eq!(a_edges.len(), 2, "exactly two edges evidenced in a.rs");
        assert!(a_edges
            .iter()
            .all(|e| e.evidence_file.as_ref() == Path::new("a.rs")));

        let b_edges: Vec<_> = g.edges_for_file(Path::new("b.rs")).collect();
        assert_eq!(b_edges.len(), 1);

        let c_edges: Vec<_> = g.edges_for_file(Path::new("c.rs")).collect();
        assert_eq!(c_edges.len(), 0, "no edges evidenced in c.rs");
    }
}

use crate::epoch::GraphEpoch;
use crate::symbol_graph::SymbolGraph;
use coregraph_core::{DirectEdge, SymbolNode};
use std::path::Path;
use std::time::SystemTime;

/// Snapshot file format version. Bump on **every** breaking change to
/// `GraphSnapshot` (added/removed fields, renamed types, semantic
/// shifts in node/edge encoding). `load_snapshot` rejects files
/// recorded under a different version with a clear error so users
/// don't see opaque bincode panics on stale `.coregraph/snapshot.bin`.
///
/// v2: removed the SCIP promotion layer — snapshots recorded under v1 may
/// contain `CompilerDerived` `Resolves` edges that no longer regenerate, so
/// they are rejected and rebuilt cleanly from source.
///
/// v3: added the documentation layer — `SymbolKind::DocComment` nodes and
/// `EdgeKind::Documents` edges. A v2 snapshot has neither; loading it under v3
/// rejects and rebuilds cleanly so the doc layer is populated.
///
/// v4: added `EdgeKind::Mentions` (doc-text intra-doc links). A v3 snapshot has
/// none; reject and rebuild so the mention edges populate.
///
/// v5: added the external-docs layer — `SymbolKind::DocSection` nodes and
/// `EdgeKind::DescribedIn` edges (Markdown ingestion). A v4 snapshot has
/// neither; reject and rebuild so they populate.
///
/// v6: added `built_at` — the wall-clock time the graph reflected its source
/// tree. The daemon persists this on idle-unload and validates it on warm
/// load (reject + rebuild if any source file is newer), so a graph restored
/// after edits made while unloaded is never served stale. A v5 snapshot has
/// no `built_at`; its bincode layout differs, so it is rejected and rebuilt.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 6;

/// Magic bytes prefixed to every snapshot file. Lets `load_snapshot`
/// distinguish a valid CoreGraph snapshot from arbitrary bincode
/// (or a truncated download) before attempting deserialization. The
/// 4-byte sequence `CGRH` makes the file identifiable in `file(1)`
/// output and hex dumps without reading further bytes.
const SNAPSHOT_MAGIC: &[u8; 4] = b"CGRH";

/// Flat, serializable representation of a SymbolGraph for disk persistence.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct GraphSnapshot {
    /// Schema version of this snapshot. See `SNAPSHOT_SCHEMA_VERSION`.
    pub schema_version: u32,
    pub epoch: GraphEpoch,
    /// Wall-clock time the graph reflected its source tree. Set by
    /// `save_snapshot`; `from_graph` leaves it at `UNIX_EPOCH` because the
    /// in-memory invalidation path (which also builds a `GraphSnapshot`) has
    /// no use for it. Validated against source mtimes on warm load.
    #[serde(default = "unix_epoch")]
    pub built_at: SystemTime,
    pub nodes: Vec<SymbolNode>,
    pub edges: Vec<DirectEdge>,
    pub next_id: u64,
}

fn unix_epoch() -> SystemTime {
    SystemTime::UNIX_EPOCH
}

impl GraphSnapshot {
    /// Build a snapshot from the current graph state.
    pub fn from_graph(graph: &SymbolGraph, epoch: GraphEpoch) -> Self {
        let nodes: Vec<SymbolNode> = graph.nodes().cloned().collect();
        let edges: Vec<DirectEdge> = graph.edges().cloned().collect();
        let next_id = nodes.iter().map(|n| n.id.0 + 1).max().unwrap_or(0);
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            epoch,
            // Placeholder: only `save_snapshot` records a meaningful value.
            built_at: SystemTime::UNIX_EPOCH,
            nodes,
            edges,
            next_id,
        }
    }

    /// Rebuild a SymbolGraph from this snapshot (preserves original SymbolIds).
    pub fn into_graph(self) -> (SymbolGraph, GraphEpoch) {
        let mut graph = SymbolGraph::new();
        for node in self.nodes {
            graph.insert_node_preserving_id(node);
        }
        for edge in self.edges {
            graph.insert_edge(edge);
        }
        (graph, self.epoch)
    }
}

/// Serialize graph + epoch to a bincode file. The file layout is:
///   bytes 0..4   — `SNAPSHOT_MAGIC`
///   bytes 4..8   — `SNAPSHOT_SCHEMA_VERSION` (u32 little-endian)
///   bytes 8..    — bincode-encoded `GraphSnapshot`
/// Reader rejects mismatches before bincode touches the body, giving
/// a clean error instead of a deserialization stack trace.
pub fn save_snapshot(
    path: &Path,
    graph: &SymbolGraph,
    epoch: GraphEpoch,
    built_at: SystemTime,
) -> anyhow::Result<()> {
    let mut snapshot = GraphSnapshot::from_graph(graph, epoch);
    snapshot.built_at = built_at;
    let bytes = bincode::serialize(&snapshot)?;
    let mut out = Vec::with_capacity(bytes.len() + 8);
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.extend_from_slice(&SNAPSHOT_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&bytes);
    std::fs::write(path, &out)?;
    Ok(())
}

/// Deserialize graph + epoch + `built_at` from a bincode file. Validates
/// the magic header and schema version before handing bytes to bincode.
pub fn load_snapshot(path: &Path) -> anyhow::Result<(SymbolGraph, GraphEpoch, SystemTime)> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 8 {
        anyhow::bail!(
            "snapshot {} is too short to contain a header (got {} bytes)",
            path.display(),
            bytes.len()
        );
    }
    if &bytes[..4] != SNAPSHOT_MAGIC {
        anyhow::bail!(
            "snapshot {} is not a CoreGraph snapshot (bad magic bytes); rebuild with `coregraph index --snapshot`",
            path.display()
        );
    }
    let on_disk_version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if on_disk_version != SNAPSHOT_SCHEMA_VERSION {
        anyhow::bail!(
            "snapshot {} uses schema v{}, but this build only reads v{}; rebuild with `coregraph index --snapshot`",
            path.display(),
            on_disk_version,
            SNAPSHOT_SCHEMA_VERSION
        );
    }
    let snapshot: GraphSnapshot = bincode::deserialize(&bytes[8..])?;
    let built_at = snapshot.built_at;
    let (graph, epoch) = snapshot.into_graph();
    Ok((graph, epoch, built_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use coregraph_core::edge::{AnalysisOrigin, Confidence};
    use coregraph_core::{DirectEdge, EdgeKind, SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn make_node(name: &str, file: &str) -> SymbolNode {
        SymbolNode::new(
            SymbolId(0),
            SymbolKind::Function,
            name,
            PathBuf::from(file),
            0,
            10,
        )
    }

    #[test]
    fn snapshot_roundtrip_nodes_and_edges() {
        let mut g = SymbolGraph::new();
        let id_a = g.insert_node(make_node("foo", "a.rs"));
        let id_b = g.insert_node(make_node("bar", "b.rs"));
        let edge = DirectEdge::new(
            id_a,
            id_b,
            EdgeKind::Calls,
            AnalysisOrigin::SyntaxMatched,
            Confidence::new(0.9),
            PathBuf::from("a.rs"),
        );
        g.insert_edge(edge);

        let epoch = GraphEpoch::zero();
        let snap = GraphSnapshot::from_graph(&g, epoch);
        assert_eq!(snap.nodes.len(), 2);
        assert_eq!(snap.edges.len(), 1);

        let (g2, e2) = snap.into_graph();
        assert_eq!(g2.node_count(), 2);
        assert_eq!(g2.edge_count(), 1);
        assert_eq!(e2, epoch);
        assert!(g2.get_node(id_a).is_some());
        assert!(g2.get_node(id_b).is_some());
    }

    #[test]
    fn save_and_load_snapshot() {
        let mut g = SymbolGraph::new();
        g.insert_node(make_node("alpha", "src/alpha.rs"));
        g.insert_node(make_node("beta", "src/beta.rs"));

        let tmp = std::env::temp_dir().join("coregraph_test_snapshot.bin");
        let epoch = GraphEpoch::zero().next();
        let built_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        save_snapshot(&tmp, &g, epoch, built_at).expect("save failed");

        let (g2, e2, b2) = load_snapshot(&tmp).expect("load failed");
        assert_eq!(g2.node_count(), 2);
        assert_eq!(e2.0, 1);
        assert_eq!(b2, built_at, "built_at must round-trip");

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn empty_graph_snapshot() {
        let g = SymbolGraph::new();
        let snap = GraphSnapshot::from_graph(&g, GraphEpoch::zero());
        assert_eq!(snap.nodes.len(), 0);
        assert_eq!(snap.edges.len(), 0);
        let (g2, _) = snap.into_graph();
        assert_eq!(g2.node_count(), 0);
    }

    #[test]
    fn load_rejects_bad_magic() {
        let tmp = std::env::temp_dir().join("coregraph_test_snapshot_bad_magic.bin");
        std::fs::write(&tmp, b"NOPE\x01\x00\x00\x00garbage").unwrap();
        let err = load_snapshot(&tmp).err().expect("expected error");
        assert!(err.to_string().contains("not a CoreGraph snapshot"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn load_rejects_wrong_schema_version() {
        // Magic is right but schema = 999 — future format we can't read.
        let tmp = std::env::temp_dir().join("coregraph_test_snapshot_bad_ver.bin");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CGRH");
        bytes.extend_from_slice(&999u32.to_le_bytes());
        bytes.extend_from_slice(b"garbage");
        std::fs::write(&tmp, &bytes).unwrap();
        let err = load_snapshot(&tmp).err().expect("expected error");
        assert!(err.to_string().contains("schema v999"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn load_rejects_truncated_header() {
        let tmp = std::env::temp_dir().join("coregraph_test_snapshot_short.bin");
        std::fs::write(&tmp, b"CGRH").unwrap();
        let err = load_snapshot(&tmp).err().expect("expected error");
        assert!(err.to_string().contains("too short"));
        std::fs::remove_file(&tmp).ok();
    }
}

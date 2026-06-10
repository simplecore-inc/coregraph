use std::path::Path;
use std::time::{Duration, SystemTime};

/// Size threshold above which we skip hashing and fall back to mtime+size.
/// 10 MiB. See `docs/change-tracking.md §7.Layer1`.
pub const MAX_HASH_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Tracked state for a single source file. A file is "actually changed"
/// when its `content_hash` differs from the remembered value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileState {
    pub content_hash: u64,
    pub mtime: SystemTime,
    pub size: u64,
}

impl FileState {
    pub fn new(content_hash: u64, mtime: SystemTime, size: u64) -> Self {
        Self {
            content_hash,
            mtime,
            size,
        }
    }
}

/// Life-cycle state of a node in the evidence-based graph.
/// See `docs/change-tracking.md §7.Layer3`.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum NodeStatus {
    /// Definition verified against current source.
    #[default]
    Verified,
    /// Source file changed; the node may no longer reflect reality, but
    /// healing has not yet confirmed its fate.
    Stale,
    /// Node never had direct evidence — introduced by inference or a
    /// cross-file fixup that was later undone.
    Assumed,
    /// Source file deleted or the definition disappeared on re-parse.
    /// Pending GC.
    Gone,
}

impl NodeStatus {
    pub fn label(&self) -> &'static str {
        match self {
            NodeStatus::Verified => "Verified",
            NodeStatus::Stale => "Stale",
            NodeStatus::Assumed => "Assumed",
            NodeStatus::Gone => "Gone",
        }
    }
}

/// Compute a content hash for file bytes using xxh3.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64(bytes)
}

/// Hash a file on disk. Returns `None` for files exceeding
/// `MAX_HASH_FILE_SIZE` so callers can fall back to mtime/size.
pub fn hash_file(path: &Path) -> std::io::Result<Option<u64>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_HASH_FILE_SIZE {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    Ok(Some(hash_bytes(&bytes)))
}

/// Sample a full `FileState` from disk. For files larger than the hash
/// threshold, `content_hash` is a pseudo-hash of `(mtime, size)` computed as
/// `nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(size)` so that
/// identical mtime+size yield identical hashes without requiring a read.
pub fn sample_file_state(path: &Path) -> std::io::Result<FileState> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let mtime = meta.modified()?;
    let content_hash = match hash_file(path)? {
        Some(h) => h,
        None => encode_mtime_size_fallback(mtime, size),
    };
    Ok(FileState {
        content_hash,
        mtime,
        size,
    })
}

fn encode_mtime_size_fallback(mtime: SystemTime, size: u64) -> u64 {
    // For files beyond the hash threshold, collapse (mtime, size) into a
    // pseudo-hash. Same (mtime, size) ⇒ same hash (good for idempotence).
    let nanos = mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos() as u64;
    nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_deterministic() {
        assert_eq!(hash_bytes(b"hello"), hash_bytes(b"hello"));
        assert_ne!(hash_bytes(b"hello"), hash_bytes(b"world"));
    }

    #[test]
    fn node_status_default_is_verified() {
        assert_eq!(NodeStatus::default(), NodeStatus::Verified);
    }

    #[test]
    fn node_status_label_roundtrips() {
        assert_eq!(NodeStatus::Verified.label(), "Verified");
        assert_eq!(NodeStatus::Stale.label(), "Stale");
        assert_eq!(NodeStatus::Assumed.label(), "Assumed");
        assert_eq!(NodeStatus::Gone.label(), "Gone");
    }

    #[test]
    fn file_state_hash_survives_clone() {
        let a = FileState::new(42, SystemTime::UNIX_EPOCH, 100);
        let b = a;
        assert_eq!(a, b);
    }
}

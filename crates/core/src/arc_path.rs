//! Serde adapter for `Arc<Path>` fields.
//!
//! `Path` is unsized and has no `Deserialize` impl, and serde's `rc` feature
//! deserializes `Arc<T>` without de-duplicating identical values. We instead
//! (de)serialize through `PathBuf`: serialize the borrowed `&Path`, and
//! deserialize into a fresh `Arc<Path>`. Snapshot loads re-intern paths when
//! nodes/edges are re-inserted into the graph, so sharing is restored there.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub fn serialize<S: Serializer>(path: &Arc<Path>, serializer: S) -> Result<S::Ok, S::Error> {
    // `Path: Serialize` (encodes as the platform string).
    Path::serialize(path.as_ref(), serializer)
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Arc<Path>, D::Error> {
    let buf = PathBuf::deserialize(deserializer)?;
    Ok(Arc::from(buf))
}

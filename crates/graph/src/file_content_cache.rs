//! Daemon-side LRU of file contents for line/column resolution.
//!
//! Keyed by absolute `PathBuf`. Entries are evicted when exceeding
//! capacity OR explicitly invalidated on reindex.
//!
//! # Key stability
//! We use `std::path::absolute` (no symlink resolution) rather than
//! `canonicalize` so that the cache key is stable even after a file is
//! deleted (e.g. during the second `get` in a deleted-file test, or
//! between an `invalidate` and a re-read). Two symlinked paths to the
//! same file will get separate cache entries — acceptable for a
//! daemon-side cache where all paths come from the same FS watcher.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct FileContentCache {
    inner: Mutex<LruCache<PathBuf, String>>,
}

/// Make `path` absolute without resolving symlinks.
///
/// Uses `std::path::absolute` (stable since Rust 1.79).  Falls back to the
/// raw path if `absolute` fails (should not happen in practice).
fn make_absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl FileContentCache {
    /// Create a new cache with the given entry capacity.
    ///
    /// # Memory
    /// `capacity` bounds the number of cached files, NOT total bytes.
    /// A single cached entry can be arbitrarily large (e.g. a 10 MB
    /// generated source file), so callers processing oversized files
    /// should size capacity accordingly (e.g. 100 for hand-written
    /// source files, lower if many large build outputs are in play).
    ///
    /// Capacity is clamped to a minimum of 1.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap())),
        }
    }

    /// Return file contents, reading from disk on miss.
    /// Returns `Ok(None)` if the file does not exist or cannot be read.
    #[must_use = "ignoring the Result discards I/O errors from reading the file"]
    pub fn get(&self, path: &Path) -> std::io::Result<Option<String>> {
        let abs = make_absolute(path);
        {
            let mut guard = self.inner.lock().unwrap();
            if let Some(s) = guard.get(&abs) {
                return Ok(Some(s.clone()));
            }
        }
        match std::fs::read_to_string(&abs) {
            Ok(contents) => {
                let mut guard = self.inner.lock().unwrap();
                guard.put(abs, contents.clone());
                Ok(Some(contents))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Drop the cached entry for `path`. Called on `reindex` for that file.
    pub fn invalidate(&self, path: &Path) {
        let abs = make_absolute(path);
        let mut guard = self.inner.lock().unwrap();
        guard.pop(&abs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// RAII guard that removes a temp file on drop so tests do not leak
    /// entries under `/tmp`. Ignores errors (the file may already be gone).
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Create a unique temp file for the test. The path includes the
    /// process id so two concurrent `cargo test` runs (e.g. cargo-watch
    /// plus a CI re-run on the same host) do not collide on a fixed name.
    fn write_tmp(name: &str, contents: &str) -> (PathBuf, TempFileGuard) {
        let path = std::env::temp_dir().join(format!(
            "coregraph-cache-test-{name}-{}",
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        let guard = TempFileGuard(path.clone());
        (path, guard)
    }

    #[test]
    fn first_get_reads_from_disk() {
        let (path, _guard) = write_tmp("first", "hello\nworld");
        let cache = FileContentCache::new(10);
        let out = cache.get(&path).unwrap().unwrap();
        assert_eq!(out, "hello\nworld");
    }

    #[test]
    fn second_get_uses_cache_even_if_file_deleted() {
        let (path, _guard) = write_tmp("cached", "persistent");
        let cache = FileContentCache::new(10);
        cache.get(&path).unwrap();
        // Intentionally delete the backing file; the guard's Drop will
        // then be a no-op (ignored NotFound).
        std::fs::remove_file(&path).unwrap();
        let out = cache.get(&path).unwrap().unwrap();
        assert_eq!(out, "persistent");
    }

    #[test]
    fn invalidate_forces_re_read() {
        let (path, _guard) = write_tmp("invalidate", "v1");
        let cache = FileContentCache::new(10);
        cache.get(&path).unwrap();
        std::fs::write(&path, "v2").unwrap();
        cache.invalidate(&path);
        let out = cache.get(&path).unwrap().unwrap();
        assert_eq!(out, "v2");
    }

    #[test]
    fn get_missing_file_returns_ok_none_not_err() {
        let cache = FileContentCache::new(10);
        let out = cache.get(Path::new("/nonexistent/path/xyz")).unwrap();
        assert!(out.is_none());
    }
}

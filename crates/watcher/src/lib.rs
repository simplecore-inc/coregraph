pub mod debounce;
pub mod git_diff;

pub use debounce::{DebouncedReceiver, PathFilter};
pub use git_diff::GitDiffStrategy;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;

/// Wraps a notify RecommendedWatcher with a DebouncedReceiver.
/// Keep this struct alive to maintain the watch — dropping it stops watching.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    pub receiver: DebouncedReceiver,
}

impl FileWatcher {
    /// Default debounce window (ms): coalesce notify events within this window
    /// before emitting a change batch.
    pub const DEFAULT_DEBOUNCE_MS: u64 = 100;

    /// Create a new watcher on `path` with the default debounce window.
    pub fn new(path: &Path) -> anyhow::Result<Self> {
        Self::with_debounce(path, Self::DEFAULT_DEBOUNCE_MS)
    }

    /// Create a new watcher with a custom debounce window.
    pub fn with_debounce(path: &Path, debounce_ms: u64) -> anyhow::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            tx.send(res).ok();
        })?;
        watcher.watch(path, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver: DebouncedReceiver::new(rx, debounce_ms),
        })
    }

    /// Create a watcher that drops every event whose path is rejected
    /// by `filter` (`filter(&path)` returning `false` → discard).
    ///
    /// Intended to bind the watcher to a `PathExcluder` from
    /// `coregraph-query` so build outputs (`target/`, `build/`),
    /// dependency caches (`node_modules/`, `.gradle/`), VCS metadata
    /// (`.git/`) and user-configured exclude patterns never trigger
    /// the daemon's reindex path. Without this filter, a single
    /// `cargo build` would emit thousands of events and stall any
    /// query behind an endless `incremental rebuild` loop.
    pub fn with_filter<F>(path: &Path, filter: F) -> anyhow::Result<Self>
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        Self::with_filter_and_debounce(path, Self::DEFAULT_DEBOUNCE_MS, filter)
    }

    /// Like [`with_filter`] but takes a custom debounce window.
    pub fn with_filter_and_debounce<F>(
        path: &Path,
        debounce_ms: u64,
        filter: F,
    ) -> anyhow::Result<Self>
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            tx.send(res).ok();
        })?;
        watcher.watch(path, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            receiver: DebouncedReceiver::with_filter(rx, debounce_ms, Box::new(filter)),
        })
    }
}

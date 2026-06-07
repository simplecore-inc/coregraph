use notify::Event;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Predicate applied to every incoming path before it reaches the
/// caller. `true` → keep, `false` → drop.
///
/// Used to bind the file watcher to the project's `PathExcluder`
/// (defined in `coregraph-query`) without creating a crate-level
/// dependency on `query` — the watcher remains a pure infra layer.
pub type PathFilter = dyn Fn(&Path) -> bool + Send + Sync + 'static;

/// Wraps a notify event Receiver and coalesces bursts of events within a debounce window.
pub struct DebouncedReceiver {
    rx: Receiver<notify::Result<Event>>,
    window: Duration,
    filter: Option<Box<PathFilter>>,
}

impl DebouncedReceiver {
    pub fn new(rx: Receiver<notify::Result<Event>>, debounce_ms: u64) -> Self {
        Self {
            rx,
            window: Duration::from_millis(debounce_ms),
            filter: None,
        }
    }

    /// Variant that drops every event whose path does not pass `filter`.
    /// Events received during the debounce window but rejected by the
    /// predicate are not just hidden from the caller — they are
    /// discarded before any downstream reindex work runs, so file
    /// trees like `target/` / `node_modules/` / `build/` never wake
    /// the daemon.
    pub fn with_filter(
        rx: Receiver<notify::Result<Event>>,
        debounce_ms: u64,
        filter: Box<PathFilter>,
    ) -> Self {
        Self {
            rx,
            window: Duration::from_millis(debounce_ms),
            filter: Some(filter),
        }
    }

    /// Block until at least one event arrives, then collect all events within
    /// the debounce window. Returns deduplicated file paths.
    pub fn next_changed_files(&self) -> Vec<PathBuf> {
        let first = match self.rx.recv() {
            Ok(Ok(event)) => event,
            _ => return vec![],
        };

        let mut paths: HashSet<PathBuf> = first.paths.into_iter().collect();
        let deadline = Instant::now() + self.window;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match self.rx.recv_timeout(remaining) {
                Ok(Ok(event)) => {
                    paths.extend(event.paths);
                }
                _ => break,
            }
        }

        if let Some(f) = &self.filter {
            paths.retain(|p| f(p));
        }
        paths.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{Event, EventKind};
    use std::sync::mpsc::channel;

    fn fake_event(paths: Vec<PathBuf>) -> notify::Result<Event> {
        Ok(Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths,
            attrs: Default::default(),
        })
    }

    #[test]
    fn collects_single_event() {
        let (tx, rx) = channel();
        let receiver = DebouncedReceiver::new(rx, 50);
        tx.send(fake_event(vec![PathBuf::from("foo.rs")])).unwrap();
        drop(tx);

        let files = receiver.next_changed_files();
        assert_eq!(files.len(), 1);
        assert!(files.contains(&PathBuf::from("foo.rs")));
    }

    #[test]
    fn deduplicates_repeated_paths() {
        let (tx, rx) = channel();
        let receiver = DebouncedReceiver::new(rx, 50);
        tx.send(fake_event(vec![PathBuf::from("a.rs")])).unwrap();
        tx.send(fake_event(vec![
            PathBuf::from("a.rs"),
            PathBuf::from("b.rs"),
        ]))
        .unwrap();
        drop(tx);

        let files = receiver.next_changed_files();
        let unique_a = files
            .iter()
            .filter(|p| p == &&PathBuf::from("a.rs"))
            .count();
        assert_eq!(unique_a, 1);
        assert!(files.contains(&PathBuf::from("b.rs")));
    }

    #[test]
    fn filter_drops_excluded_paths() {
        // The regression guard for the watcher-level exclude wiring.
        // Without the filter, every event in `target/` would reach the
        // daemon and trigger an `incremental rebuild` loop.
        let (tx, rx) = channel();
        let receiver = DebouncedReceiver::with_filter(
            rx,
            50,
            Box::new(|p: &Path| {
                // Keep only events outside a simulated `target/` tree.
                !p.starts_with("target")
            }),
        );
        tx.send(fake_event(vec![
            PathBuf::from("src/main.rs"),
            PathBuf::from("target/debug/foo.rs"),
            PathBuf::from("target/release/bar.rs"),
        ]))
        .unwrap();
        drop(tx);

        let files = receiver.next_changed_files();
        assert_eq!(files.len(), 1, "only src/main.rs should survive");
        assert!(files.contains(&PathBuf::from("src/main.rs")));
    }

    #[test]
    fn filter_allows_everything_when_predicate_is_permissive() {
        let (tx, rx) = channel();
        let receiver = DebouncedReceiver::with_filter(rx, 50, Box::new(|_p: &Path| true));
        tx.send(fake_event(vec![
            PathBuf::from("a.rs"),
            PathBuf::from("b.rs"),
        ]))
        .unwrap();
        drop(tx);

        let files = receiver.next_changed_files();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn filter_can_reject_all_events() {
        let (tx, rx) = channel();
        let receiver = DebouncedReceiver::with_filter(rx, 50, Box::new(|_p: &Path| false));
        tx.send(fake_event(vec![PathBuf::from("a.rs")])).unwrap();
        drop(tx);

        let files = receiver.next_changed_files();
        assert!(files.is_empty());
    }
}

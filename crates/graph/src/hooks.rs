use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::SymbolGraph;

/// The phase at which a hook fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookKind {
    /// Fires before analysis (graph construction)
    PreAnalysis,
    /// Fires after analysis completes
    PostAnalysis,
}

/// Pre-analysis callback: receives the root path. Return Err to abort the build.
pub type PreHookFn = Arc<dyn Fn(&Path) -> anyhow::Result<()> + Send + Sync>;

/// Post-analysis callback: receives the built graph for inspection/reporting.
pub type PostHookFn = Arc<dyn Fn(&SymbolGraph) -> anyhow::Result<()> + Send + Sync>;

/// Executable callback variant. Matches the HookKind of its entry.
#[derive(Clone)]
pub enum HookCallback {
    Pre(PreHookFn),
    Post(PostHookFn),
}

impl std::fmt::Debug for HookCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookCallback::Pre(_) => write!(f, "HookCallback::Pre(<fn>)"),
            HookCallback::Post(_) => write!(f, "HookCallback::Post(<fn>)"),
        }
    }
}

/// A named hook with a kind, description, and optional executable callback.
#[derive(Debug, Clone)]
pub struct HookEntry {
    pub name: String,
    pub kind: HookKind,
    pub description: String,
    /// Executable callback. `None` means metadata-only (display in `plugin list`).
    pub callback: Option<HookCallback>,
}

impl HookEntry {
    /// Create a metadata-only entry (no callback). Useful for `plugin list`.
    pub fn new(name: impl Into<String>, kind: HookKind, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            description: description.into(),
            callback: None,
        }
    }

    /// Create a pre-analysis hook with an executable callback.
    pub fn pre<F>(name: impl Into<String>, description: impl Into<String>, f: F) -> Self
    where
        F: Fn(&Path) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            kind: HookKind::PreAnalysis,
            description: description.into(),
            callback: Some(HookCallback::Pre(Arc::new(f))),
        }
    }

    /// Create a post-analysis hook with an executable callback.
    pub fn post<F>(name: impl Into<String>, description: impl Into<String>, f: F) -> Self
    where
        F: Fn(&SymbolGraph) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            kind: HookKind::PostAnalysis,
            description: description.into(),
            callback: Some(HookCallback::Post(Arc::new(f))),
        }
    }
}

/// Registry for pre/post analysis hooks.
/// Hooks are registered by name; duplicates are replaced.
#[derive(Default, Debug, Clone)]
pub struct HookRegistry {
    hooks: HashMap<String, HookEntry>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a hook. If a hook with the same name exists, it is replaced.
    pub fn register(&mut self, hook: HookEntry) {
        self.hooks.insert(hook.name.clone(), hook);
    }

    /// Remove a hook by name. Returns true if removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        self.hooks.remove(name).is_some()
    }

    /// List all registered hooks, sorted by name.
    pub fn list(&self) -> Vec<&HookEntry> {
        let mut entries: Vec<&HookEntry> = self.hooks.values().collect();
        entries.sort_by_key(|h| h.name.as_str());
        entries
    }

    /// List hooks of a specific kind.
    pub fn list_by_kind(&self, kind: HookKind) -> Vec<&HookEntry> {
        self.list().into_iter().filter(|h| h.kind == kind).collect()
    }

    /// Returns true if a hook with the given name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.hooks.contains_key(name)
    }

    /// Total count of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Invoke every registered PreAnalysis hook in name-sorted order.
    /// Returns the first error encountered, otherwise Ok.
    pub fn fire_pre(&self, root: &Path) -> anyhow::Result<()> {
        for entry in self.list_by_kind(HookKind::PreAnalysis) {
            if let Some(HookCallback::Pre(f)) = &entry.callback {
                f(root).map_err(|e| {
                    anyhow::anyhow!("pre-analysis hook '{}' failed: {}", entry.name, e)
                })?;
            }
        }
        Ok(())
    }

    /// Invoke every registered PostAnalysis hook in name-sorted order.
    /// Returns the first error encountered, otherwise Ok.
    pub fn fire_post(&self, graph: &SymbolGraph) -> anyhow::Result<()> {
        for entry in self.list_by_kind(HookKind::PostAnalysis) {
            if let Some(HookCallback::Post(f)) = &entry.callback {
                f(graph).map_err(|e| {
                    anyhow::anyhow!("post-analysis hook '{}' failed: {}", entry.name, e)
                })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    #[test]
    fn register_and_list() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::new(
            "my-hook",
            HookKind::PreAnalysis,
            "test hook",
        ));
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("my-hook"));
    }

    #[test]
    fn unregister_removes_hook() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::new("h", HookKind::PostAnalysis, "desc"));
        assert!(reg.unregister("h"));
        assert!(!reg.contains("h"));
        assert!(!reg.unregister("h"));
    }

    #[test]
    fn list_by_kind_filters_correctly() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::new("pre1", HookKind::PreAnalysis, ""));
        reg.register(HookEntry::new("post1", HookKind::PostAnalysis, ""));
        reg.register(HookEntry::new("pre2", HookKind::PreAnalysis, ""));
        let pre = reg.list_by_kind(HookKind::PreAnalysis);
        assert_eq!(pre.len(), 2);
        let post = reg.list_by_kind(HookKind::PostAnalysis);
        assert_eq!(post.len(), 1);
    }

    #[test]
    fn duplicate_registration_replaces() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::new("dup", HookKind::PreAnalysis, "first"));
        reg.register(HookEntry::new("dup", HookKind::PostAnalysis, "second"));
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.list()[0].kind, HookKind::PostAnalysis);
    }

    #[test]
    fn empty_registry_is_empty() {
        let reg = HookRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn fire_pre_invokes_callback() {
        let counter = StdArc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::pre("count-pre", "counter", move |_p| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));

        reg.fire_pre(Path::new(".")).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fire_post_invokes_callback() {
        let counter = StdArc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::post("count-post", "counter", move |_g| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));

        let g = SymbolGraph::new();
        reg.fire_post(&g).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fire_pre_metadata_only_hook_no_error() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::new("meta", HookKind::PreAnalysis, "no callback"));
        reg.fire_pre(Path::new(".")).unwrap();
    }

    #[test]
    fn pre_hook_error_propagates() {
        let mut reg = HookRegistry::new();
        reg.register(HookEntry::pre("fails", "always fails", |_p| {
            Err(anyhow::anyhow!("boom"))
        }));
        let result = reg.fire_pre(Path::new("."));
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("fails"));
        assert!(msg.contains("boom"));
    }
}

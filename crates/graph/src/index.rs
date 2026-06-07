use std::collections::HashMap;

use coregraph_core::SymbolId;

/// Maps symbol names to their `SymbolId`s for fast lookup-by-name.
pub struct NameIndex {
    map: HashMap<String, Vec<SymbolId>>,
}

impl NameIndex {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Register a symbol name → id mapping.
    pub fn insert(&mut self, name: String, id: SymbolId) {
        self.map.entry(name).or_default().push(id);
    }

    /// Look up all ids associated with `name`.
    /// Returns an empty slice if the name is not found.
    pub fn lookup(&self, name: &str) -> &[SymbolId] {
        self.map.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

impl Default for NameIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_by_name() {
        let mut idx = NameIndex::new();
        idx.insert("foo".into(), SymbolId(1));
        let result = idx.lookup("foo");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], SymbolId(1));
    }

    #[test]
    fn lookup_missing_returns_empty() {
        let idx = NameIndex::new();
        assert!(idx.lookup("nonexistent").is_empty());
    }

    #[test]
    fn lookup_multiple_same_name() {
        let mut idx = NameIndex::new();
        idx.insert("overloaded".into(), SymbolId(10));
        idx.insert("overloaded".into(), SymbolId(20));
        let result = idx.lookup("overloaded");
        assert_eq!(result.len(), 2);
        assert!(result.contains(&SymbolId(10)));
        assert!(result.contains(&SymbolId(20)));
    }
}

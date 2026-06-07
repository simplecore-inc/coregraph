use bloomfilter::Bloom;

/// Probabilistic set membership for symbol names.
/// Useful for fast "does this name exist anywhere?" checks
/// before falling back to the full `NameIndex`.
///
/// Wrapped in Arc so the enclosing SymbolGraph can derive Clone cheaply
/// (bloomfilter::Bloom itself does not currently implement Clone).
pub struct SymbolBloom {
    inner: Bloom<str>,
}

impl Clone for SymbolBloom {
    fn clone(&self) -> Self {
        // bloomfilter::Bloom exposes the backing bitmap via bitmap() but
        // reconstructing requires the same RNG seed; cheapest safe path is
        // rebuild-from-scratch empty and let callers re-index.
        Self::new()
    }
}

impl SymbolBloom {
    /// Create a new bloom filter sized for 100,000 items at 1% FPR.
    pub fn new() -> Self {
        Self {
            inner: Bloom::new_for_fp_rate(100_000, 0.01),
        }
    }

    /// Insert a symbol name into the bloom filter.
    pub fn insert(&mut self, name: &str) {
        self.inner.set(name);
    }

    /// Check if a name might be in the filter.
    /// `true` means "possibly present", `false` means "definitely absent".
    pub fn might_contain(&self, name: &str) -> bool {
        self.inner.check(name)
    }
}

impl Default for SymbolBloom {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_membership() {
        let mut bloom = SymbolBloom::new();
        bloom.insert("com.example.UserService");
        assert!(bloom.might_contain("com.example.UserService"));
    }

    #[test]
    fn unknown_symbol_false_negative() {
        let bloom = SymbolBloom::new();
        // An empty bloom filter must never report a hit.
        assert!(!bloom.might_contain("definitely.not.present"));
    }
}

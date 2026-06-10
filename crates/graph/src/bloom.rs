use bloomfilter::Bloom;

/// Probabilistic set membership for symbol names.
/// Useful for fast "does this name exist anywhere?" checks.
///
/// `Clone` copies the backing bitmap and hash keys (via
/// `bloomfilter::Bloom::from_existing`), so a cloned `SymbolBloom` — and
/// therefore a cloned `SymbolGraph` — keeps the populated filter and
/// `might_contain`'s "`false` = definitely absent" contract holds across
/// clones. We can't derive `Clone`: the derived impl for `Bloom<str>` would
/// demand `str: Clone`, which `str` (unsized) does not satisfy.
pub struct SymbolBloom {
    inner: Bloom<str>,
}

impl Clone for SymbolBloom {
    fn clone(&self) -> Self {
        Self {
            inner: Bloom::from_existing(
                &self.inner.bitmap(),
                self.inner.number_of_bits(),
                self.inner.number_of_hash_functions(),
                self.inner.sip_keys(),
            ),
        }
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

    #[test]
    fn clone_preserves_membership() {
        // A cloned filter must keep the original's contents, otherwise a
        // cloned SymbolGraph would report `false` ("definitely absent") for
        // names the file actually defines, breaking `might_contain`'s contract.
        let mut bloom = SymbolBloom::new();
        bloom.insert("com.example.UserService");
        let cloned = bloom.clone();
        assert!(cloned.might_contain("com.example.UserService"));
    }
}

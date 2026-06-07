use crate::epoch::GraphEpoch;
use std::collections::HashMap;

/// Cache keyed by (query_hash, epoch). Stale entries (epoch < current) are evicted
/// on demand via invalidate_before.
pub struct EpochKeyedCache<V> {
    store: HashMap<(u64, u64), V>,
}

impl<V> EpochKeyedCache<V> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }

    /// Look up a cached value for the given query hash at the given epoch.
    pub fn get(&self, query_hash: u64, epoch: GraphEpoch) -> Option<&V> {
        self.store.get(&(query_hash, epoch.0))
    }

    /// Store a result keyed by (query_hash, epoch).
    pub fn insert(&mut self, query_hash: u64, epoch: GraphEpoch, value: V) {
        self.store.insert((query_hash, epoch.0), value);
    }

    /// Remove all entries from epochs strictly before the given epoch.
    pub fn invalidate_before(&mut self, epoch: GraphEpoch) {
        self.store.retain(|(_, e), _| *e >= epoch.0);
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

impl<V> Default for EpochKeyedCache<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_and_miss() {
        let mut cache: EpochKeyedCache<String> = EpochKeyedCache::new();
        let epoch0 = GraphEpoch::zero();
        cache.insert(42, epoch0, "result".to_string());

        assert_eq!(cache.get(42, epoch0), Some(&"result".to_string()));
        assert_eq!(cache.get(99, epoch0), None);
    }

    #[test]
    fn cache_epoch_boundary() {
        let mut cache: EpochKeyedCache<u32> = EpochKeyedCache::new();
        let epoch0 = GraphEpoch::zero();
        let epoch1 = epoch0.next();
        cache.insert(1, epoch0, 10);
        cache.insert(1, epoch1, 20);

        assert_eq!(cache.get(1, epoch0), Some(&10));
        assert_eq!(cache.get(1, epoch1), Some(&20));
    }

    #[test]
    fn invalidate_before_removes_stale() {
        let mut cache: EpochKeyedCache<u32> = EpochKeyedCache::new();
        let epoch0 = GraphEpoch::zero();
        let epoch1 = epoch0.next();
        let epoch2 = epoch1.next();
        cache.insert(1, epoch0, 10);
        cache.insert(2, epoch1, 20);
        cache.insert(3, epoch2, 30);

        cache.invalidate_before(epoch2);
        assert_eq!(cache.get(1, epoch0), None);
        assert_eq!(cache.get(2, epoch1), None);
        assert_eq!(cache.get(3, epoch2), Some(&30));
        assert_eq!(cache.len(), 1);
    }
}

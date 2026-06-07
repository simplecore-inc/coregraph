/// Monotonically incrementing version counter for the graph.
/// Incremented on every invalidation+heal cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GraphEpoch(pub u64);

impl GraphEpoch {
    pub fn zero() -> Self {
        Self(0)
    }
    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_starts_at_zero() {
        assert_eq!(GraphEpoch::zero().0, 0);
    }

    #[test]
    fn epoch_increments() {
        let e = GraphEpoch::zero().next().next();
        assert_eq!(e.0, 2);
    }
}

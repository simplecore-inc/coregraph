//! Small text utilities shared across analysis layers.

/// Cache-friendly two-row iterative Levenshtein, intended for short
/// identifiers and API paths.
///
/// NOTE: coregraph-graph keeps its own private `levenshtein_bounded`
/// (early-exit variant) for hot paths — see crates/graph/src/symbol_graph.rs —
/// extend that one for cutoff-aware lookups, this one for exact distances.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let n = a_bytes.len();
    let m = b_bytes.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = usize::from(a_bytes[i - 1] != b_bytes[j - 1]);
            curr[j] = std::cmp::min(
                std::cmp::min(prev[j] + 1, curr[j - 1] + 1),
                prev[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::levenshtein;

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("cards", "card"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}

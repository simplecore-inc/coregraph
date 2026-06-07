use crate::types::{QueryResult, TokenBudget};

/// Rough token estimate: 1 token ≈ 4 characters.
pub fn estimate_tokens(result: &QueryResult) -> usize {
    if result.matches.is_empty() {
        return 0;
    }
    let content: usize = result.matches.iter().map(|n| n.name.len() + 10).sum();
    (result.symbol_name.len() + content).div_ceil(4)
}

/// Returns true if the result fits within the available token budget.
pub fn fits_budget(result: &QueryResult, budget: &TokenBudget) -> bool {
    estimate_tokens(result) <= budget.available()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::QueryResult;

    #[test]
    fn estimate_tokens_empty() {
        let r = QueryResult::empty("x".to_string());
        assert_eq!(estimate_tokens(&r), 0);
    }

    #[test]
    fn budget_fits_returns_true() {
        let r = QueryResult::empty("x".to_string());
        let budget = crate::types::TokenBudget {
            max_tokens: 8000,
            reserve_tokens: 500,
        };
        assert!(fits_budget(&r, &budget));
    }

    #[test]
    fn budget_exceeded_still_returns_bool() {
        let r = QueryResult::empty("x".to_string());
        let budget = crate::types::TokenBudget {
            max_tokens: 0,
            reserve_tokens: 0,
        };
        // 0 tokens available, empty result has 0 tokens — fits (0 <= 0)
        assert!(fits_budget(&r, &budget));
    }
}

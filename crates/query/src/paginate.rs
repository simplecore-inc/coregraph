use crate::types::QueryResult;

/// Slice `result.matches` to the requested page window.
pub fn paginate(result: &QueryResult, page: usize, page_size: usize) -> QueryResult {
    if page_size == 0 {
        return result.clone();
    }
    let start = page * page_size;
    let matches = result
        .matches
        .iter()
        .skip(start)
        .take(page_size)
        .cloned()
        .collect();
    QueryResult {
        symbol_name: result.symbol_name.clone(),
        matches,
        edges: result.edges.clone(),
        total_nodes: result.total_nodes,
        total_edges: result.total_edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::QueryResult;
    use coregraph_core::{SymbolId, SymbolKind, SymbolNode};
    use std::path::PathBuf;

    fn make_result(n: usize) -> QueryResult {
        let mut r = QueryResult::empty("Test".to_string());
        for i in 0..n {
            r.matches.push(SymbolNode::new(
                SymbolId(i as u64),
                SymbolKind::Function,
                format!("fn_{i}"),
                PathBuf::from("src/lib.rs"),
                i as u32,
                (i + 1) as u32,
            ));
        }
        r.total_nodes = n;
        r
    }

    #[test]
    fn paginate_first_page() {
        let r = make_result(10);
        let p = paginate(&r, 0, 3);
        assert_eq!(p.matches.len(), 3);
        assert_eq!(p.matches[0].name, "fn_0");
    }

    #[test]
    fn paginate_second_page() {
        let r = make_result(10);
        let p = paginate(&r, 1, 3);
        assert_eq!(p.matches.len(), 3);
        assert_eq!(p.matches[0].name, "fn_3");
    }

    #[test]
    fn paginate_last_partial_page() {
        let r = make_result(10);
        let p = paginate(&r, 3, 3);
        assert_eq!(p.matches.len(), 1);
        assert_eq!(p.matches[0].name, "fn_9");
    }

    #[test]
    fn paginate_preserves_total() {
        let r = make_result(10);
        let p = paginate(&r, 0, 3);
        assert_eq!(p.total_nodes, 10);
    }
}

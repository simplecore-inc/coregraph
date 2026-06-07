use axum::{
    extract::{Query, State},
    Json,
};
use coregraph_core::SymbolId;
use coregraph_query::{compute_impact, query_symbol};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::AppState;

/// Health check response.
#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub symbol_count: usize,
}

/// GET /health — server liveness check.
pub async fn health(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let symbol_count = state.graph.read().map(|g| g.node_count()).unwrap_or(0);
    Json(HealthResponse {
        status: "ok".to_string(),
        version: state.version.clone(),
        symbol_count,
    })
}

/// Request body for POST /query.
#[derive(Deserialize)]
pub struct QueryRequest {
    pub name: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// Response body for POST /query.
#[derive(Serialize)]
pub struct QueryResponse {
    pub name: String,
    pub count: usize,
    pub symbols: Vec<String>,
}

/// POST /query — look up symbols by name.
pub async fn query_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QueryRequest>,
) -> Json<QueryResponse> {
    let graph = state.graph.read().expect("graph lock poisoned");
    let result = query_symbol(&req.name, &graph);
    let symbols: Vec<String> = result
        .matches
        .iter()
        .take(req.limit)
        .map(|n| n.name.clone())
        .collect();
    Json(QueryResponse {
        name: req.name,
        count: symbols.len(),
        symbols,
    })
}

/// Request body for POST /batch.
#[derive(Deserialize)]
pub struct BatchRequest {
    pub queries: Vec<String>,
}

/// Response body for POST /batch.
#[derive(Serialize)]
pub struct BatchResponse {
    pub results: Vec<QueryResponse>,
}

/// POST /batch — run multiple symbol queries in one request.
pub async fn batch_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BatchRequest>,
) -> Json<BatchResponse> {
    let graph = state.graph.read().expect("graph lock poisoned");
    let results = req
        .queries
        .into_iter()
        .map(|name| {
            let result = query_symbol(&name, &graph);
            let symbols: Vec<String> = result.matches.iter().map(|n| n.name.clone()).collect();
            QueryResponse {
                name: name.clone(),
                count: symbols.len(),
                symbols,
            }
        })
        .collect();
    Json(BatchResponse { results })
}

/// GET /api/query?symbol=X&page=1&budget=4000  (docs/cli.md §12.5)
pub async fn api_query(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let symbol = params.get("symbol").cloned().unwrap_or_default();
    let page: usize = params.get("page").and_then(|v| v.parse().ok()).unwrap_or(0);
    let page_size: usize = params
        .get("page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let budget: usize = params
        .get("budget")
        .and_then(|v| v.parse().ok())
        .unwrap_or(8000);

    let graph = state.graph.read().expect("graph lock poisoned");
    let result = query_symbol(&symbol, &graph);
    let total = result.matches.len();
    let start = page * page_size;
    let end = (start + page_size).min(total);
    let matches: Vec<_> = result
        .matches
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "file": n.file.display().to_string(),
                "span_start": n.span_start,
                "span_end": n.span_end,
            })
        })
        .collect();
    Json(serde_json::json!({
        "query": symbol,
        "matches": matches,
        "pagination": {
            "page": page,
            "page_size": page_size,
            "total": total,
            "total_pages": if page_size == 0 { 1 } else { total.div_ceil(page_size).max(1) },
        },
        "budget": {
            "advertised_total": budget,
            "effective_total": (budget as f32 * 0.7) as usize,
            "estimation_method": "byte_approximation",
        }
    }))
}

/// GET /api/expand?node=<id>&budget=2000  (docs/cli.md §12.5)
pub async fn api_expand(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let id: u64 = params.get("node").and_then(|v| v.parse().ok()).unwrap_or(0);
    let budget: usize = params
        .get("budget")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let sid = SymbolId(id);

    let graph = state.graph.read().expect("graph lock poisoned");
    let Some(node) = graph.get_node(sid) else {
        return Json(serde_json::json!({"error": "node not found", "id": id}));
    };
    let incoming: Vec<_> = graph
        .edges()
        .filter(|e| e.to == sid)
        .map(|e| {
            serde_json::json!({
                "from": e.from,
                "kind": format!("{:?}", e.kind),
                "confidence": e.confidence.0,
                "trust": format!("{:?}", e.origin),
                "origin": format!("{:?}", e.origin),
                "trust_model": format!("{:?}", e.trust_model()),
                "stale_evidence_count": e.stale_evidence_count,
                "current_confidence": e.current_confidence(),
            })
        })
        .collect();
    let outgoing: Vec<_> = graph
        .edges()
        .filter(|e| e.from == sid)
        .map(|e| {
            serde_json::json!({
                "to": e.to,
                "kind": format!("{:?}", e.kind),
                "confidence": e.confidence.0,
                "trust": format!("{:?}", e.origin),
                "origin": format!("{:?}", e.origin),
                "trust_model": format!("{:?}", e.trust_model()),
                "stale_evidence_count": e.stale_evidence_count,
                "current_confidence": e.current_confidence(),
            })
        })
        .collect();
    Json(serde_json::json!({
        "node": {
            "id": node.id,
            "name": node.name,
            "kind": format!("{:?}", node.kind),
            "file": node.file.display().to_string(),
            "span_start": node.span_start,
            "span_end": node.span_end,
        },
        "incoming": incoming,
        "outgoing": outgoing,
        "budget": {
            "advertised_total": budget,
            "effective_total": (budget as f32 * 0.7) as usize,
            "estimation_method": "byte_approximation",
        }
    }))
}

/// GET /api/impact?symbol=X&depth=3  (docs/cli.md §12.5)
pub async fn api_impact(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let symbol = params.get("symbol").cloned().unwrap_or_default();
    let depth: usize = params
        .get("depth")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    let graph = state.graph.read().expect("graph lock poisoned");
    let Some(seed) = graph.nodes().find(|n| n.name == symbol).cloned() else {
        return Json(serde_json::json!({
            "symbol": symbol,
            "error": "not found",
            "reachable": 0,
        }));
    };
    let result = compute_impact(&graph, seed.id, depth);
    let nodes: Vec<_> = result
        .reachable
        .iter()
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
                "file": n.file.display().to_string(),
            })
        })
        .collect();
    Json(serde_json::json!({
        "symbol": symbol,
        "depth": result.depth_reached,
        "reachable_count": result.reachable.len(),
        "edge_count": result.edges.len(),
        "nodes": nodes,
    }))
}

/// GET /api/source?file=X&line=42&context=10  (docs/cli.md §12.5)
pub async fn api_source(
    State(_state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let file = params.get("file").cloned().unwrap_or_default();
    let line: usize = params.get("line").and_then(|v| v.parse().ok()).unwrap_or(1);
    let context: usize = params
        .get("context")
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);

    let source = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            return Json(serde_json::json!({
                "file": file,
                "error": format!("cannot read: {}", e),
            }));
        }
    };
    let lines: Vec<&str> = source.lines().collect();
    let total = lines.len();
    let start = line.saturating_sub(context).max(1);
    let end = (line + context).min(total);
    let snippet: Vec<_> = (start..=end)
        .map(|n| {
            serde_json::json!({
                "line": n,
                "text": lines.get(n - 1).copied().unwrap_or(""),
                "is_target": n == line,
            })
        })
        .collect();
    Json(serde_json::json!({
        "file": file,
        "target_line": line,
        "context_lines": context,
        "total_lines": total,
        "snippet": snippet,
    }))
}

#[cfg(test)]
mod tests {
    use crate::{create_app, AppState};
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::util::ServiceExt;

    fn test_app() -> axum::Router {
        create_app(AppState::new())
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn query_endpoint_returns_json() {
        let app = test_app();
        let body = r#"{"name":"main"}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn batch_endpoint_returns_results_array() {
        let app = test_app();
        let body = r#"{"queries":["foo","bar"]}"#;
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/batch")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

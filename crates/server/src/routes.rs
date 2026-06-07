use axum::{routing::get, routing::post, Router};
use std::sync::Arc;

use crate::{handlers, AppState};

/// Register all routes on the router.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Simple endpoints
        .route("/health", get(handlers::health))
        .route("/query", post(handlers::query_handler))
        .route("/batch", post(handlers::batch_handler))
        // Docs §12.5 — REST over the same cached graph
        .route("/api/query", get(handlers::api_query))
        .route("/api/expand", get(handlers::api_expand))
        .route("/api/impact", get(handlers::api_impact))
        .route("/api/source", get(handlers::api_source))
        .with_state(state)
}

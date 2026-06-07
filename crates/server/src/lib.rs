pub mod handlers;
pub mod routes;

use axum::Router;
use coregraph_graph::SymbolGraph;
use std::sync::{Arc, RwLock};

/// Shared application state passed to all handlers.
#[derive(Clone)]
pub struct AppState {
    /// Version string from Cargo.toml
    pub version: String,
    /// Symbol graph shared across requests.
    pub graph: Arc<RwLock<SymbolGraph>>,
}

impl AppState {
    /// Create an AppState with an empty graph (suitable for tests).
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            graph: Arc::new(RwLock::new(SymbolGraph::new())),
        }
    }

    /// Create an AppState pre-loaded with the given graph.
    pub fn with_graph(graph: SymbolGraph) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            graph: Arc::new(RwLock::new(graph)),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the axum Router with all routes attached.
pub fn create_app(state: AppState) -> Router {
    routes::build_router(Arc::new(state))
}

/// Start the HTTP server on the given address.
pub async fn serve(addr: &str, state: AppState) -> anyhow::Result<()> {
    let app = create_app(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_has_version() {
        let state = AppState::new();
        assert!(!state.version.is_empty());
    }

    #[test]
    fn create_app_returns_router() {
        let state = AppState::new();
        let _router = create_app(state);
    }

    #[test]
    fn app_state_with_graph_empty() {
        let graph = SymbolGraph::new();
        let state = AppState::with_graph(graph);
        assert_eq!(state.graph.read().unwrap().node_count(), 0);
    }
}

//! OpenAI- and Ollama-compatible HTTP server. Phase 4 implementation.

pub mod openai;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;

use hexrun_core::Engine;

#[derive(Clone)]
pub struct ServerState {
    pub engine: Option<Arc<Engine>>,
}

pub fn router(state: ServerState) -> Router {
    Router::new()
        .merge(openai::routes())
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, state: ServerState) -> Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hexrun server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

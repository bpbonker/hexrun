//! OpenAI- and Ollama-compatible HTTP server. Phase 4 implementation.
//!
//! Holds a single loaded `Engine` (and therefore a single resident NPU
//! dialog) for the lifetime of the server. Concurrent requests are
//! serialized via a `std::sync::Mutex` because Qualcomm's Genie does not
//! support concurrent queries on a single dialog handle.

pub mod ollama;
pub mod openai;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::Router;
use hexrun_core::Engine;

/// Shared state for the HTTP handlers.
#[derive(Clone, Default)]
pub struct ServerState {
    /// The loaded inference engine, wrapped in a Mutex so concurrent
    /// requests serialize their queries. None when the server starts
    /// without a preloaded model (will return 503 Service Unavailable
    /// until a model is loaded).
    pub engine: Option<Arc<Mutex<Engine>>>,
    /// Model name reported in `/v1/models` and `/api/tags`.
    pub model_name: Option<String>,
}

/// Build the axum router with all hexrun endpoints.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .merge(openai::routes())
        .merge(ollama::routes())
        .route("/healthz", axum::routing::get(healthz))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

/// Run the HTTP server until shutdown. Blocks the calling task.
pub async fn serve(addr: SocketAddr, state: ServerState) -> Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hexrun server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

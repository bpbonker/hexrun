//! `/v1/chat/completions`, `/v1/completions`, `/v1/models`, plus Ollama-compat
//! `/api/pull` and `/api/tags`. SSE streaming via `tokio::sync::mpsc`. Phase 4.

use axum::{routing::get, Router};

use crate::ServerState;

pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/api/tags", get(api_tags))
}

async fn list_models() -> &'static str {
    "{\"object\":\"list\",\"data\":[]}"
}

async fn api_tags() -> &'static str {
    "{\"models\":[]}"
}

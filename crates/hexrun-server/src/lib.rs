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
use std::time::SystemTime;

use anyhow::Result;
use axum::extract::State;
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum::Router;
use hexrun_core::Engine;
use tower_http::cors::{Any, CorsLayer};

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
    /// Wall-clock time the server started; surfaced via /healthz.
    pub started_at: Option<SystemTime>,
    /// Optional bearer token. When set, all `/v1/*` and `/api/*`
    /// requests must include `Authorization: Bearer <token>`. Health
    /// and root endpoints stay unauthenticated.
    pub auth_token: Option<String>,
}

/// Build the axum router with all hexrun endpoints.
///
/// Adds permissive CORS (any origin, any method, common headers) so
/// browser-based clients (Open WebUI, custom UIs) can hit the server
/// from a different origin. When `state.auth_token` is set, a
/// bearer-token middleware guards `/v1/*` and `/api/*` endpoints.
pub fn router(state: ServerState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            header::ACCEPT,
            header::CACHE_CONTROL,
        ]);

    let api = Router::new()
        .merge(openai::routes())
        .merge(ollama::routes())
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .merge(api)
        .route("/healthz", axum::routing::get(healthz))
        .route("/", axum::routing::get(root_index))
        .layer(cors)
        .with_state(state)
}

/// Bearer-token middleware. Skips when `state.auth_token` is None.
async fn require_auth(
    State(state): State<ServerState>,
    req: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let Some(expected) = state.auth_token.as_ref() else {
        return next.run(req).await;
    };
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let ok = matches!(header, Some(h) if h.strip_prefix("Bearer ").map(|s| s.trim()) == Some(expected.as_str()));
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer realm=\"hexrun\"")],
            Json(serde_json::json!({
                "error": {
                    "message": "missing or invalid Authorization: Bearer <token> header",
                    "type": "auth_required"
                }
            })),
        )
            .into_response();
    }
    next.run(req).await
}

async fn healthz(State(state): State<ServerState>) -> Json<serde_json::Value> {
    let uptime_secs = state
        .started_at
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let model_loaded = state.engine.is_some();
    Json(serde_json::json!({
        "status": if model_loaded { "ready" } else { "no_model_loaded" },
        "model": state.model_name,
        "uptime_seconds": uptime_secs,
        "auth": state.auth_token.is_some(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn root_index() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "hexrun",
        "endpoints": {
            "openai": ["GET /v1/models", "POST /v1/chat/completions"],
            "ollama": ["GET /api/tags", "POST /api/generate", "POST /api/chat"],
            "health": ["GET /healthz"],
        },
    }))
}

/// Run the HTTP server until shutdown. Blocks the calling task.
pub async fn serve(addr: SocketAddr, state: ServerState) -> Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hexrun server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

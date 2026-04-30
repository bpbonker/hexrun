//! OpenAI- and Ollama-compatible HTTP server. Phase 4 implementation.
//!
//! Holds a single loaded `Engine` (and therefore a single resident NPU
//! dialog) for the lifetime of the server. Concurrent requests are
//! serialized via a `std::sync::Mutex` because Qualcomm's Genie does not
//! support concurrent queries on a single dialog handle.

pub mod ollama;
pub mod openai;

use std::net::SocketAddr;
use std::sync::Arc;
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
///
/// Concurrency model: the loaded `Engine` is shared as an `Arc` and the
/// concurrent-inference-of-one rule is enforced by a `tokio::sync::Semaphore`
/// with a single permit. Handlers `try_acquire` the permit before
/// spawning the blocking inference task; if it's already held they return
/// HTTP 429 instead of blocking on a `Mutex`. The permit is released
/// when the spawned task ends (its `OwnedSemaphorePermit` is dropped).
#[derive(Clone)]
pub struct ServerState {
    /// The loaded inference engine. Shared read-only across requests;
    /// serialization is handled by `inference_permit` below. None when
    /// the server starts without a preloaded model (endpoints return
    /// 503 until a model is loaded).
    pub engine: Option<Arc<Engine>>,
    /// Single-permit semaphore that serializes concurrent inference
    /// requests. Held by whichever request task is currently running
    /// inside `spawn_blocking`; everyone else gets a 429 with
    /// `Retry-After`.
    pub inference_permit: Arc<tokio::sync::Semaphore>,
    /// Model name reported in `/v1/models` and `/api/tags`.
    pub model_name: Option<String>,
    /// Wall-clock time the server started; surfaced via /healthz.
    pub started_at: Option<SystemTime>,
    /// Optional bearer token. When set, all `/v1/*` and `/api/*`
    /// requests must include `Authorization: Bearer <token>`. Health
    /// and root endpoints stay unauthenticated.
    pub auth_token: Option<String>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self {
            engine: None,
            inference_permit: Arc::new(tokio::sync::Semaphore::new(1)),
            model_name: None,
            started_at: None,
            auth_token: None,
        }
    }
}

impl ServerState {
    /// Build a fresh server state with a single-permit inference semaphore.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder helper used by `hexrun serve --model <name>`.
    pub fn with_engine(mut self, engine: Arc<Engine>, model_name: String) -> Self {
        self.engine = Some(engine);
        self.model_name = Some(model_name);
        self
    }
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

/// Run the HTTP server until shutdown.
///
/// Listens for SIGINT (Ctrl+C). On signal, drains in-flight requests
/// before closing the listener — clients with an active inference task
/// get to finish; new connections are refused.
pub async fn serve(addr: SocketAddr, state: ServerState) -> Result<()> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "hexrun server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("hexrun server stopped cleanly");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::warn!(error = %e, "failed to install Ctrl+C handler; server will not shut down gracefully");
        // If the handler can't install, await forever rather than shut down.
        std::future::pending::<()>().await;
    }
    tracing::info!("Ctrl+C received; draining in-flight requests");
}

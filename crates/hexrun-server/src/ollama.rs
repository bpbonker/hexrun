//! Ollama-compatible HTTP endpoints (`/api/*`).
//!
//! Minimal compatibility surface: enough that Open WebUI and similar
//! Ollama-aware clients see a model and can chat with it. We mirror the
//! NDJSON streaming protocol Ollama uses.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use futures::stream::Stream;
use hexrun_core::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{error, info};

use crate::ServerState;

/// `/api/*` route subtree.
pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/api/tags", get(api_tags))
        .route("/api/generate", post(api_generate))
        .route("/api/chat", post(api_chat))
}

#[derive(Serialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Serialize)]
struct TagModel {
    name: String,
    modified_at: String,
    size: u64,
    digest: String,
}

async fn api_tags(State(state): State<ServerState>) -> Json<TagsResponse> {
    let models = match state.model_name {
        Some(name) => vec![TagModel {
            name,
            modified_at: now_iso8601(),
            size: 0,
            digest: String::new(),
        }],
        None => vec![],
    };
    Json(TagsResponse { models })
}

#[derive(Deserialize)]
struct GenerateRequest {
    #[serde(default)]
    model: Option<String>,
    prompt: String,
    #[serde(default = "default_stream")]
    stream: bool,
}

#[derive(Deserialize)]
struct ChatRequest {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<ChatMessage>,
    #[serde(default = "default_stream")]
    stream: bool,
}

#[derive(Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

fn default_stream() -> bool {
    true
}

#[derive(Serialize)]
struct GenerateChunk<'a> {
    model: &'a str,
    created_at: String,
    response: &'a str,
    done: bool,
}

#[derive(Serialize)]
struct ChatChunk<'a> {
    model: &'a str,
    created_at: String,
    message: ChatChunkMessage<'a>,
    done: bool,
}

#[derive(Serialize)]
struct ChatChunkMessage<'a> {
    role: &'static str,
    content: &'a str,
}

async fn api_generate(
    State(state): State<ServerState>,
    Json(req): Json<GenerateRequest>,
) -> Response {
    let engine = match state.engine.clone() {
        Some(e) => e,
        None => return no_model_loaded().into_response(),
    };
    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "hexrun".to_string());
    info!(model = %model_name, stream = req.stream, "ollama /api/generate");

    if req.stream {
        let stream = ndjson_stream(engine, req.prompt, model_name, ResponseShape::Generate);
        ndjson_response(stream).into_response()
    } else {
        match run_blocking(engine, req.prompt).await {
            Ok(text) => Json(serde_json::json!({
                "model": model_name,
                "created_at": now_iso8601(),
                "response": text,
                "done": true,
            }))
            .into_response(),
            Err(msg) => inference_error(&msg),
        }
    }
}

async fn api_chat(State(state): State<ServerState>, Json(req): Json<ChatRequest>) -> Response {
    let engine = match state.engine.clone() {
        Some(e) => e,
        None => return no_model_loaded().into_response(),
    };
    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "hexrun".to_string());
    let user_msg = match req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
    {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "no user message in request"})),
            )
                .into_response()
        }
    };
    info!(model = %model_name, stream = req.stream, "ollama /api/chat");

    if req.stream {
        let stream = ndjson_stream(engine, user_msg, model_name, ResponseShape::Chat);
        ndjson_response(stream).into_response()
    } else {
        match run_blocking(engine, user_msg).await {
            Ok(text) => Json(serde_json::json!({
                "model": model_name,
                "created_at": now_iso8601(),
                "message": { "role": "assistant", "content": text },
                "done": true,
            }))
            .into_response(),
            Err(msg) => inference_error(&msg),
        }
    }
}

#[derive(Clone, Copy)]
enum ResponseShape {
    Generate,
    Chat,
}

async fn run_blocking(engine: Arc<Mutex<Engine>>, prompt: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let engine = engine
            .lock()
            .map_err(|_| "engine mutex poisoned".to_string())?;
        engine.generate(&prompt).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("inference task panicked: {e}"))?
}

fn inference_error(msg: &str) -> Response {
    error!(error = %msg, "inference failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": msg,
        })),
    )
        .into_response()
}

fn ndjson_stream(
    engine: Arc<Mutex<Engine>>,
    user_prompt: String,
    model_name: String,
    shape: ResponseShape,
) -> impl Stream<Item = Result<axum::body::Bytes, Infallible>> {
    let (tx, rx) = mpsc::channel::<StreamItem>(64);
    let tx_clone = tx.clone();
    tokio::task::spawn_blocking(move || {
        let engine = match engine.lock() {
            Ok(e) => e,
            Err(_) => {
                let _ = tx_clone.blocking_send(StreamItem::Error("engine mutex poisoned".into()));
                return;
            }
        };
        let res = engine.generate_streaming(&user_prompt, |chunk| {
            let _ = tx_clone.blocking_send(StreamItem::Content(chunk.to_string()));
        });
        match res {
            Ok(()) => {
                let _ = tx_clone.blocking_send(StreamItem::Done);
            }
            Err(e) => {
                error!(error = %e, "ollama streaming inference failed");
                let _ = tx_clone.blocking_send(StreamItem::Error(e.to_string()));
            }
        }
    });
    drop(tx);

    ReceiverStream::new(rx).map(move |item| {
        let now = now_iso8601();
        let bytes = match (shape, item) {
            (ResponseShape::Generate, StreamItem::Content(s)) => {
                let chunk = GenerateChunk {
                    model: &model_name,
                    created_at: now,
                    response: &s,
                    done: false,
                };
                let mut v = serde_json::to_vec(&chunk).unwrap_or_default();
                v.push(b'\n');
                v
            }
            (ResponseShape::Generate, StreamItem::Done) => {
                let chunk = GenerateChunk {
                    model: &model_name,
                    created_at: now,
                    response: "",
                    done: true,
                };
                let mut v = serde_json::to_vec(&chunk).unwrap_or_default();
                v.push(b'\n');
                v
            }
            (ResponseShape::Chat, StreamItem::Content(s)) => {
                let chunk = ChatChunk {
                    model: &model_name,
                    created_at: now,
                    message: ChatChunkMessage {
                        role: "assistant",
                        content: &s,
                    },
                    done: false,
                };
                let mut v = serde_json::to_vec(&chunk).unwrap_or_default();
                v.push(b'\n');
                v
            }
            (ResponseShape::Chat, StreamItem::Done) => {
                let chunk = ChatChunk {
                    model: &model_name,
                    created_at: now,
                    message: ChatChunkMessage {
                        role: "assistant",
                        content: "",
                    },
                    done: true,
                };
                let mut v = serde_json::to_vec(&chunk).unwrap_or_default();
                v.push(b'\n');
                v
            }
            (_, StreamItem::Error(msg)) => {
                let payload = serde_json::json!({"error": msg});
                let mut v = serde_json::to_vec(&payload).unwrap_or_default();
                v.push(b'\n');
                v
            }
        };
        Ok(axum::body::Bytes::from(bytes))
    })
}

fn ndjson_response<S>(stream: S) -> Response
where
    S: Stream<Item = Result<axum::body::Bytes, Infallible>> + Send + 'static,
{
    let body = axum::body::Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/x-ndjson")
        .body(body)
        .unwrap_or_default()
}

fn no_model_loaded() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "no model loaded; start the server with --model <name>",
        })),
    )
}

fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Lightweight formatting; not strictly RFC-3339 nanosecond precision but
    // good enough for Ollama clients that just want a "looks like a date" string.
    let days = secs / 86400;
    let rem = secs % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    // Anchor on Unix epoch (1970-01-01) and approximate via days. Real
    // calendar conversion can come later when we have a chrono/time dep.
    let _ = days;
    format!("1970-01-01T{h:02}:{m:02}:{s:02}Z")
}

enum StreamItem {
    Content(String),
    Done,
    Error(String),
}

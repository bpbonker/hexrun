//! OpenAI-compatible Chat Completions endpoints (`/v1/*`).
//!
//! Implements the streaming and non-streaming `/v1/chat/completions`
//! endpoint using SSE for the streaming case. Concrete enough that
//! existing OpenAI client libraries (and chat UIs that speak OpenAI)
//! can point at the server unchanged.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
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

/// `/v1/*` route subtree.
pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
}

#[derive(Serialize)]
struct ModelsResponse {
    object: &'static str,
    data: Vec<ModelObject>,
}

#[derive(Serialize)]
struct ModelObject {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: &'static str,
}

async fn list_models(State(state): State<ServerState>) -> Json<ModelsResponse> {
    let created = unix_now();
    let data = match state.model_name {
        Some(name) => vec![ModelObject {
            id: name,
            object: "model",
            created,
            owned_by: "hexrun",
        }],
        None => vec![],
    };
    Json(ModelsResponse {
        object: "list",
        data,
    })
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ChatRequest {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    // Sampler params are accepted for OpenAI client compatibility but
    // are currently ignored — sampling is configured in the bundle's
    // `genie_config.json`. Wiring runtime overrides is a later phase.
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatResponse<'a> {
    id: String,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<ChatChoice<'a>>,
}

#[derive(Serialize)]
struct ChatChoice<'a> {
    index: u32,
    message: ChatMessageOut<'a>,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChatMessageOut<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatChunk<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<ChatChunkChoice<'a>>,
}

#[derive(Serialize)]
struct ChatChunkChoice<'a> {
    index: u32,
    delta: ChatDelta<'a>,
    finish_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct ChatDelta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
}

async fn chat_completions(
    State(state): State<ServerState>,
    Json(req): Json<ChatRequest>,
) -> axum::response::Response {
    let engine = match state.engine.clone() {
        Some(e) => e,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": {
                        "message": "no model loaded; start the server with --model <name>",
                        "type": "no_model_loaded"
                    }
                })),
            )
                .into_response();
        }
    };

    // Pull the most recent user message; ignore everything else for the
    // current Phase 4 implementation. Multi-turn conversation will be
    // wired through Genie's KV-cache rewind in a later phase.
    let user_prompt = match latest_user_message(&req.messages) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "message": "no user message in request", "type": "bad_request" }
                })),
            )
                .into_response();
        }
    };

    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "hexrun".to_string());
    let id = format!("chatcmpl-{}", request_id());
    let created = unix_now();

    info!(
        model = %model_name,
        stream = req.stream,
        user_msg_chars = user_prompt.chars().count(),
        "chat completion request"
    );

    if req.stream {
        chat_completions_streaming(engine, user_prompt, model_name, id, created)
            .await
            .into_response()
    } else {
        chat_completions_blocking(engine, user_prompt, model_name, id, created)
            .await
            .into_response()
    }
}

async fn chat_completions_blocking(
    engine: Arc<Mutex<Engine>>,
    user_prompt: String,
    model_name: String,
    id: String,
    created: u64,
) -> axum::response::Response {
    let join = tokio::task::spawn_blocking(move || {
        let engine = engine
            .lock()
            .map_err(|_| "engine mutex poisoned".to_string())?;
        engine.generate(&user_prompt).map_err(|e| e.to_string())
    })
    .await;

    let text = match join {
        Ok(Ok(text)) => text,
        Ok(Err(msg)) => {
            error!(error = %msg, "chat completion failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "message": msg, "type": "inference_error" }
                })),
            )
                .into_response();
        }
        Err(join_err) => {
            error!(error = %join_err, "chat completion task panicked");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": {
                        "message": format!("inference task panicked: {join_err}"),
                        "type": "inference_error"
                    }
                })),
            )
                .into_response();
        }
    };

    let body = ChatResponse {
        id,
        object: "chat.completion",
        created,
        model: &model_name,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessageOut {
                role: "assistant",
                content: &text,
            },
            finish_reason: "stop",
        }],
    };
    (StatusCode::OK, Json(body)).into_response()
}

async fn chat_completions_streaming(
    engine: Arc<Mutex<Engine>>,
    user_prompt: String,
    model_name: String,
    id: String,
    created: u64,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<StreamItem>(64);

    // Send the role-only opening delta first so OpenAI clients see the
    // assistant role before any content tokens.
    let _ = tx.try_send(StreamItem::Role);

    let id_clone = id.clone();
    let model_clone = model_name.clone();
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
                error!(error = %e, "streaming inference failed");
                let _ = tx_clone.blocking_send(StreamItem::Error(e.to_string()));
            }
        }
    });
    drop(tx);

    let stream = ReceiverStream::new(rx).map(move |item| {
        let event = match item {
            StreamItem::Role => {
                let chunk = ChatChunk {
                    id: &id_clone,
                    object: "chat.completion.chunk",
                    created,
                    model: &model_clone,
                    choices: vec![ChatChunkChoice {
                        index: 0,
                        delta: ChatDelta {
                            role: Some("assistant"),
                            content: None,
                        },
                        finish_reason: None,
                    }],
                };
                Event::default().json_data(chunk).unwrap_or_default()
            }
            StreamItem::Content(s) => {
                let chunk = ChatChunk {
                    id: &id_clone,
                    object: "chat.completion.chunk",
                    created,
                    model: &model_clone,
                    choices: vec![ChatChunkChoice {
                        index: 0,
                        delta: ChatDelta {
                            role: None,
                            content: Some(&s),
                        },
                        finish_reason: None,
                    }],
                };
                Event::default().json_data(chunk).unwrap_or_default()
            }
            StreamItem::Done => {
                let chunk = ChatChunk {
                    id: &id_clone,
                    object: "chat.completion.chunk",
                    created,
                    model: &model_clone,
                    choices: vec![ChatChunkChoice {
                        index: 0,
                        delta: ChatDelta {
                            role: None,
                            content: None,
                        },
                        finish_reason: Some("stop"),
                    }],
                };
                Event::default().json_data(chunk).unwrap_or_default()
            }
            StreamItem::Error(msg) => {
                let payload =
                    serde_json::json!({ "error": { "message": msg, "type": "inference_error" } });
                Event::default().json_data(payload).unwrap_or_default()
            }
        };
        Ok(event)
    });

    // Append a final `data: [DONE]` event after the stream closes so
    // OpenAI clients see the end-of-stream marker they expect.
    let stream = stream.chain(tokio_stream::once(Ok(Event::default().data("[DONE]"))));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

enum StreamItem {
    Role,
    Content(String),
    Done,
    Error(String),
}

fn latest_user_message(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn request_id() -> String {
    // Cheap unique-ish id without pulling in a uuid dep.
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

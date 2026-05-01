//! OpenAI-compatible Chat Completions endpoints (`/v1/*`).
//!
//! Implements the streaming and non-streaming `/v1/chat/completions`
//! endpoint using SSE for the streaming case. Concrete enough that
//! existing OpenAI client libraries (and chat UIs that speak OpenAI)
//! can point at the server unchanged.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use futures::stream::Stream;
use npurun_core::{ChatMessage as CoreChatMessage, ChatRole, Engine};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::OwnedSemaphorePermit;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

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
        Some(name) => {
            // Advertise both the bare name and the `:latest`-tagged form.
            // OpenAI clients want the bare name; Ollama-aware UIs probing
            // the OpenAI surface (Open WebUI's compatibility mode) want
            // the tagged form.
            vec![
                ModelObject {
                    id: name.clone(),
                    object: "model",
                    created,
                    owned_by: "npurun",
                },
                ModelObject {
                    id: format!("{name}:latest"),
                    object: "model",
                    created,
                    owned_by: "npurun",
                },
            ]
        }
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
    /// OpenAI JSON-mode hint. When `{"type": "json_object"}` is sent,
    /// the server injects a system instruction asking the model to
    /// respond with valid JSON only. This is *not* constrained
    /// sampling — the model can still emit invalid JSON. Clients
    /// should retry on parse failure, same as against OpenAI.
    #[serde(default)]
    response_format: Option<ResponseFormat>,
}

#[derive(Deserialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
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

    // Convert the request's messages to the engine's canonical
    // representation. The engine builds the full transcript via the
    // bundle's chat template and sends it to Genie with
    // `SentenceCode::Rewind`, so multi-turn replay reuses the KV
    // cache prefix instead of re-prefilling from scratch.
    let mut messages: Vec<CoreChatMessage> = req
        .messages
        .iter()
        .filter_map(|m| {
            let role = match m.role.as_str() {
                "system" => ChatRole::System,
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => return None, // tool/function roles are silently dropped
            };
            Some(CoreChatMessage {
                role,
                content: m.content.clone(),
            })
        })
        .collect();

    if let Some(fmt) = &req.response_format {
        if fmt.kind == "json_object" {
            messages = augment_for_json_mode(messages);
        }
    }

    if !messages.iter().any(|m| m.role == ChatRole::User) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "message": "no user message in request", "type": "bad_request" }
            })),
        )
            .into_response();
    }

    // Reserve the inference slot before doing any heavy work. If the
    // server is already running another request, return 429.
    let permit = match state.inference_permit.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!(model = ?state.model_name, "inference slot busy, returning 429");
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "1")],
                Json(serde_json::json!({
                    "error": {
                        "message": "another inference request is in progress; retry shortly",
                        "type": "busy"
                    }
                })),
            )
                .into_response();
        }
    };

    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "npurun".to_string());
    let id = format!("chatcmpl-{}", request_id());
    let created = unix_now();

    info!(
        model = %model_name,
        stream = req.stream,
        turns = messages.len(),
        "chat completion request"
    );

    if req.stream {
        chat_completions_streaming(engine, permit, messages, model_name, id, created)
            .await
            .into_response()
    } else {
        chat_completions_blocking(engine, permit, messages, model_name, id, created)
            .await
            .into_response()
    }
}

async fn chat_completions_blocking(
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    messages: Vec<CoreChatMessage>,
    model_name: String,
    id: String,
    created: u64,
) -> axum::response::Response {
    let join = tokio::task::spawn_blocking(move || {
        let _permit = permit; // released when this closure returns
        engine.generate_chat(&messages).map_err(|e| e.to_string())
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
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    messages: Vec<CoreChatMessage>,
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
        let _permit = permit; // released when the closure returns
        let res = engine.generate_chat_streaming(&messages, |chunk| {
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

/// Honour `response_format: {"type": "json_object"}` by augmenting the
/// system message (or prepending one) with an instruction to emit valid
/// JSON only. This is a passthrough hint, not constrained sampling —
/// the model can still produce invalid JSON. Clients should retry on
/// parse failure, mirroring OpenAI's own JSON-mode contract.
fn augment_for_json_mode(messages: Vec<CoreChatMessage>) -> Vec<CoreChatMessage> {
    const HINT: &str = "You must respond with valid JSON only. Do not include explanations, prose, code fences, or markdown — output only the raw JSON object.";

    let mut out = Vec::with_capacity(messages.len() + 1);
    let mut had_system = false;
    for msg in messages {
        if matches!(msg.role, ChatRole::System) && !had_system {
            had_system = true;
            out.push(CoreChatMessage {
                role: ChatRole::System,
                content: format!("{}\n\n{HINT}", msg.content),
            });
        } else {
            out.push(msg);
        }
    }
    if !had_system {
        out.insert(
            0,
            CoreChatMessage {
                role: ChatRole::System,
                content: HINT.to_string(),
            },
        );
    }
    out
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

#[cfg(test)]
mod json_mode_tests {
    use super::{augment_for_json_mode, ChatRole, CoreChatMessage as Msg};

    fn user(s: &str) -> Msg {
        Msg {
            role: ChatRole::User,
            content: s.to_string(),
        }
    }
    fn system(s: &str) -> Msg {
        Msg {
            role: ChatRole::System,
            content: s.to_string(),
        }
    }

    #[test]
    fn prepends_system_when_none_present() {
        let out = augment_for_json_mode(vec![user("give me an object")]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].role, ChatRole::System));
        assert!(out[0].content.contains("valid JSON"));
    }

    #[test]
    fn merges_into_existing_system() {
        let out = augment_for_json_mode(vec![system("be terse"), user("hi")]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].role, ChatRole::System));
        assert!(out[0].content.starts_with("be terse"));
        assert!(out[0].content.contains("valid JSON"));
    }

    #[test]
    fn merges_into_first_system_only() {
        // Two system messages is rare but a malformed client could send
        // it. Augmenting only the first keeps the rest verbatim.
        let out = augment_for_json_mode(vec![system("first"), system("second"), user("hi")]);
        assert_eq!(out.len(), 3);
        assert!(out[0].content.contains("valid JSON"));
        assert_eq!(out[1].content, "second");
    }
}

//! OpenAI-compatible Chat Completions endpoints (`/v1/*`).
//!
//! Implements the streaming and non-streaming `/v1/chat/completions`
//! endpoint using SSE for the streaming case. Concrete enough that
//! existing OpenAI client libraries (and chat UIs that speak OpenAI)
//! can point at the server unchanged.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

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
    /// OpenAI tool-calling spec. When `tools` is non-empty, the
    /// server injects a tool-use system instruction and parses the
    /// model's reply for `<tool_call>{...}</tool_call>` markers.
    /// MVP: only the non-streaming path supports tools — passing
    /// `stream: true` with tools returns 400. Phi 3.5 Mini wasn't
    /// trained for tool use, so quality is highly prompt-dependent;
    /// Llama 3.1 / Qwen 2.5 do better.
    #[serde(default)]
    tools: Option<Vec<ToolSpec>>,
    #[serde(default)]
    #[allow(dead_code)] // reserved for future "auto"/"required"/specific-name behaviour
    tool_choice: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize, Clone)]
struct ToolSpec {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    kind: String, // "function"
    function: ToolFunction,
}

#[derive(Deserialize, Clone)]
struct ToolFunction {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<serde_json::Value>,
}

#[derive(Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: Option<String>,
    /// For `role: "tool"` messages, this links back to the assistant's
    /// `tool_calls[].id` from the previous turn.
    #[serde(default)]
    tool_call_id: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallOut>>,
}

#[derive(Serialize)]
struct ToolCallOut {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str, // "function"
    function: ToolCallFunctionOut,
}

#[derive(Serialize)]
struct ToolCallFunctionOut {
    name: String,
    /// Stringified JSON, per the OpenAI spec — clients re-parse it.
    arguments: String,
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
            let content = m.content.clone().unwrap_or_default();
            match m.role.as_str() {
                "system" => Some(CoreChatMessage {
                    role: ChatRole::System,
                    content,
                }),
                "user" => Some(CoreChatMessage {
                    role: ChatRole::User,
                    content,
                }),
                "assistant" => Some(CoreChatMessage {
                    role: ChatRole::Assistant,
                    content,
                }),
                // OpenAI's tool-result role gets folded into a synthetic
                // user message tagged with the tool_call_id, so the model
                // sees the result inline in the next prefill.
                "tool" => {
                    let id = m.tool_call_id.clone().unwrap_or_default();
                    Some(CoreChatMessage {
                        role: ChatRole::User,
                        content: format!(
                            "<tool_result tool_call_id=\"{id}\">{content}</tool_result>"
                        ),
                    })
                }
                _ => None,
            }
        })
        .collect();

    // Reject streaming + tools combo for now — partial tool_call deltas
    // are non-trivial and clients that use tools usually accept blocking.
    let has_tools = req.tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
    if has_tools && req.stream {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "message": "tool calls require stream=false in this version",
                    "type": "bad_request"
                }
            })),
        )
            .into_response();
    }

    if has_tools {
        messages = augment_for_tools(messages, req.tools.as_deref().unwrap_or(&[]));
    }

    if let Some(fmt) = &req.response_format {
        if fmt.kind == "json_object" {
            messages = augment_for_json_mode(messages);
        }
    }

    // Trim oldest turns when the transcript would exceed Genie's
    // context window. Estimated tokens via `chars/4 + 8` per message;
    // keeps the system message and as many recent turns as fit under
    // DEFAULT_INPUT_TOKEN_BUDGET. Without this, a long chat hits
    // ERROR_QUERY_FAILED and the dialog handle is permanently bricked.
    let (trimmed, was_trimmed) = trim_messages_for_context(messages, DEFAULT_INPUT_TOKEN_BUDGET);
    messages = trimmed;
    if was_trimmed {
        warn!(
            kept = messages.len(),
            "trimmed conversation history to fit Genie context window"
        );
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

    // Reset the dialog so each chat-completions request prefills fresh
    // from the full transcript, with `SentenceCode::Complete`. The
    // `Rewind` fast-path requires the new transcript to be a strict
    // prefix-extension of what's in the KV cache; small differences
    // (whitespace, end-of-turn markers re-encoded slightly differently
    // when a client sends back its locally-stored assistant text)
    // force divergence and Genie eventually returns ERROR_QUERY_FAILED.
    // Trade-off: TTFT per turn becomes full-prefill of the transcript
    // (~200 ms – 1 s for typical chats) instead of "rewind + small
    // delta". Reliability over speed until the rewind path is robust
    // against client-side roundtripping.
    if let Err(e) = engine.reset_dialog() {
        error!(error = %e, "dialog reset before request failed");
    }

    if req.stream {
        chat_completions_streaming(
            engine,
            permit,
            messages,
            model_name,
            id,
            created,
            req.max_tokens,
        )
        .await
        .into_response()
    } else {
        chat_completions_blocking(
            engine,
            permit,
            messages,
            model_name,
            id,
            created,
            req.max_tokens,
            has_tools,
        )
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
    _max_tokens: Option<u32>,
    tools_enabled: bool,
) -> axum::response::Response {
    // max_tokens is currently only enforced on the streaming path
    // (via the chunk-size budget + signal_abort). Blocking responses
    // accumulate the full reply before sending; threading max_tokens
    // through Genie's sampler is tracked separately in the roadmap.
    let watchdog_done_tx = spawn_inference_watchdog(engine.clone());

    let join = tokio::task::spawn_blocking(move || {
        let _permit = permit; // released when this closure returns
        let res = engine.generate_chat(&messages).map_err(|e| e.to_string());
        // Reset on error so the next request starts on a clean KV cache.
        // ERROR_QUERY_FAILED is sticky on the dialog otherwise.
        if res.is_err() {
            if let Err(reset_err) = engine.reset_dialog() {
                error!(error = %reset_err, "dialog reset after inference error also failed");
            }
        }
        res
    })
    .await;
    let _ = watchdog_done_tx.send(());

    let text = match join {
        Ok(Ok(text)) => text,
        Ok(Err(msg)) => {
            error!(error = %msg, "chat completion failed; dialog reset");
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

    // Scan the model's reply for `<tool_call>{...}</tool_call>` markers.
    // If any are found, return them as OpenAI-style `tool_calls` so the
    // client can dispatch the function and round-trip the result back as
    // a `role: "tool"` message. When the request didn't pass `tools` we
    // skip the parse entirely — that keeps normal chat replies that
    // happen to contain stray angle-bracket text from being misread.
    let parsed = if tools_enabled {
        parse_tool_calls(&text)
    } else {
        None
    };
    let (content_out, tool_calls_out, finish_reason): (Option<&str>, _, &'static str) = match parsed
    {
        Some(calls) => (None, Some(calls), "tool_calls"),
        None => (Some(text.as_str()), None, "stop"),
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
                content: content_out,
                tool_calls: tool_calls_out,
            },
            finish_reason,
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
    max_tokens: Option<u32>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<StreamItem>(64);

    // Send the role-only opening delta first so OpenAI clients see the
    // assistant role before any content tokens.
    let _ = tx.try_send(StreamItem::Role);

    let id_clone = id.clone();
    let model_clone = model_name.clone();
    let tx_clone = tx.clone();

    // `aborted` flips to `true` when either the client disconnects (the
    // mpsc receiver was dropped, so `blocking_send` errors) or the
    // server-side `max_tokens` budget is hit. After flipping, the
    // closure stops forwarding chunks AND calls `engine.signal_abort()`
    // so Genie returns from the next inter-token check instead of
    // running to natural EOS. Without this, a client that closes its
    // tab leaves a zombie inference holding the single inference permit
    // until Genie hits EOS or context-end (could be minutes).
    let engine_for_closure = engine.clone();
    let max_chars = max_tokens.map(|t| (t as usize).saturating_mul(4)); // ~1 token = 1-5 chars; budget loose
    let watchdog_done_tx = spawn_inference_watchdog(engine.clone());

    tokio::task::spawn_blocking(move || {
        let _permit = permit; // released when the closure returns
        let mut total_chars: usize = 0;
        let mut aborted = false;
        let res = engine_for_closure.generate_chat_streaming(&messages, |chunk| {
            if aborted {
                return;
            }
            total_chars = total_chars.saturating_add(chunk.len());
            if let Some(limit) = max_chars {
                if total_chars > limit {
                    aborted = true;
                    if let Err(e) = engine_for_closure.signal_abort() {
                        error!(error = %e, "signal_abort (max_tokens) failed");
                    }
                    return;
                }
            }
            if tx_clone
                .blocking_send(StreamItem::Content(chunk.to_string()))
                .is_err()
            {
                // Client disconnected. Tell Genie to wind down.
                aborted = true;
                if let Err(e) = engine_for_closure.signal_abort() {
                    error!(error = %e, "signal_abort (client disconnect) failed");
                }
            }
        });
        match res {
            Ok(()) => {
                let _ = tx_clone.blocking_send(StreamItem::Done);
            }
            Err(e) => {
                error!(error = %e, "streaming inference failed; resetting dialog");
                // Reset the dialog so the next request starts on a clean
                // KV cache. Without this, ERROR_QUERY_FAILED is sticky:
                // every subsequent request fails on the broken dialog.
                if let Err(reset_err) = engine_for_closure.reset_dialog() {
                    error!(error = %reset_err, "dialog reset after inference error also failed");
                }
                let _ = tx_clone.blocking_send(StreamItem::Error(e.to_string()));
            }
        }
        if aborted {
            // After abort the dialog is in a transitional state; reset
            // so the next request starts clean.
            if let Err(e) = engine_for_closure.reset_dialog() {
                error!(error = %e, "dialog reset after abort failed");
            }
        }
        // Cancel the watchdog so it doesn't fire on an idle dialog.
        let _ = watchdog_done_tx.send(());
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

/// Coarse character-based budget that approximates Genie's token-level
/// context limit. We default to 3000 "tokens" of input (4096 context
/// minus ~512 reserved for the response and some slack); each message
/// is estimated at `chars/4 + 8` (8 tokens of chat-template overhead
/// per message). When the conversation exceeds the budget, drop the
/// oldest non-system messages from the front until it fits — exactly
/// what real chat clients do, but here on the server so every client
/// benefits without each having to implement it. Without trimming,
/// long chats hit Genie's hard limit and the dialog enters
/// ERROR_QUERY_FAILED, after which even `GenieDialog_reset` doesn't
/// recover (the handle is bricked until the process restarts).
const DEFAULT_INPUT_TOKEN_BUDGET: usize = 3000;

/// Wall-clock cap for any single inference. After this, a watchdog
/// task calls `engine.signal_abort()` so the inference returns and
/// the inference permit is released. Without this, a model that
/// runs to its internal max-tokens (Phi 3.5 has been observed
/// generating 4096 tokens of "(Note: …)" continuations) holds the
/// permit for 5+ minutes and looks like a hung server to clients.
const INFERENCE_WALL_CLOCK_MAX: Duration = Duration::from_secs(60);

/// Spawn a watchdog that aborts the inference after the wall-clock
/// cap. Returns a oneshot sender; the caller signals on it when the
/// inference is done so the watchdog cancels itself instead of firing
/// against an idle dialog.
fn spawn_inference_watchdog(engine: Arc<Engine>) -> tokio::sync::oneshot::Sender<()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(INFERENCE_WALL_CLOCK_MAX) => {
                warn!(
                    timeout_secs = INFERENCE_WALL_CLOCK_MAX.as_secs(),
                    "inference exceeded wall-clock cap; signalling Genie to abort"
                );
                if let Err(e) = engine.signal_abort() {
                    error!(error = %e, "watchdog signal_abort failed");
                }
            }
            _ = rx => {
                // inference finished cleanly; nothing to do.
            }
        }
    });
    tx
}

fn estimate_message_tokens(msg: &CoreChatMessage) -> usize {
    msg.content.len() / 4 + 8
}

fn trim_messages_for_context(
    messages: Vec<CoreChatMessage>,
    max_input_tokens: usize,
) -> (Vec<CoreChatMessage>, bool) {
    let total: usize = messages.iter().map(estimate_message_tokens).sum();
    if total <= max_input_tokens {
        return (messages, false);
    }

    let mut system_msg = None;
    let mut rest = Vec::new();
    for msg in messages {
        if matches!(msg.role, ChatRole::System) && system_msg.is_none() {
            system_msg = Some(msg);
        } else {
            rest.push(msg);
        }
    }

    let sys_tokens = system_msg
        .as_ref()
        .map(estimate_message_tokens)
        .unwrap_or(0);
    let mut budget = max_input_tokens.saturating_sub(sys_tokens);

    // Keep messages from the back (most recent) until the budget is
    // exhausted. Always keep the last message if at all possible —
    // that's the user's current question.
    let mut kept_rev = Vec::new();
    for msg in rest.into_iter().rev() {
        let cost = estimate_message_tokens(&msg);
        if cost > budget && !kept_rev.is_empty() {
            break;
        }
        budget = budget.saturating_sub(cost);
        kept_rev.push(msg);
    }
    kept_rev.reverse();

    let mut result = Vec::new();
    if let Some(sys) = system_msg {
        result.push(sys);
    }
    result.extend(kept_rev);
    (result, true)
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

/// Honour OpenAI-style `tools` by injecting a system instruction that
/// describes each tool and tells the model to emit a single
/// `<tool_call>{...}</tool_call>` line when it wants to call one. This
/// is prompt-based, not constrained sampling — Phi 3.5 Mini was not
/// trained for tool use and will ignore this instruction more often
/// than a Llama-3.1-Instruct or Qwen-2.5 model would. Adequate as an
/// MVP; clients should still validate parsed JSON before dispatching.
fn augment_for_tools(messages: Vec<CoreChatMessage>, tools: &[ToolSpec]) -> Vec<CoreChatMessage> {
    if tools.is_empty() {
        return messages;
    }
    let mut hint = String::new();
    hint.push_str(
        "You have access to the following tools. When you need to call one, output a single line of EXACTLY this form, and nothing else:\n",
    );
    hint.push_str(
        "<tool_call>{\"name\": \"<tool_name>\", \"arguments\": {<json args>}}</tool_call>\n\n",
    );
    hint.push_str("Example — to call get_current_time with no arguments:\n");
    hint.push_str("<tool_call>{\"name\": \"get_current_time\", \"arguments\": {}}</tool_call>\n\n");
    hint.push_str("Rules:\n");
    hint.push_str("- The tag name MUST be literally \"tool_call\", not the tool name.\n");
    hint.push_str("- Do not wrap the line in code fences or add prose.\n");
    hint.push_str("- After the user replies with the tool result, you may answer normally.\n\n");
    hint.push_str("Available tools:\n");
    for t in tools {
        let params = t
            .function
            .parameters
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        let desc = t.function.description.as_deref().unwrap_or("");
        hint.push_str(&format!(
            "- {}: {}\n  parameters: {}\n",
            t.function.name, desc, params
        ));
    }

    let mut out = Vec::with_capacity(messages.len() + 1);
    let mut had_system = false;
    for msg in messages {
        if matches!(msg.role, ChatRole::System) && !had_system {
            had_system = true;
            out.push(CoreChatMessage {
                role: ChatRole::System,
                content: format!("{}\n\n{hint}", msg.content),
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
                content: hint,
            },
        );
    }
    out
}

/// Extract tool-call blocks from the model's reply. Canonical form is
/// `<tool_call>{"name": ..., "arguments": ...}</tool_call>`, but small
/// models (Phi 3.5 in particular) often substitute the tool name as the
/// tag, e.g. `<get_current_time>{...}</get_current_time>`. The parser
/// scans every well-formed `<word>...</word>` pair: if the body parses
/// as a JSON object containing a `name` field, that wins; otherwise the
/// tag name is treated as the tool name and the body as its arguments.
/// Malformed blocks are silently skipped — falling back to plain
/// content beats failing the whole completion.
fn parse_tool_calls(text: &str) -> Option<Vec<ToolCallOut>> {
    // Bare top-level JSON shortcut: small models (Phi 3.5) often drop
    // the wrapping tag entirely and emit just the call object. Only
    // accept this when the trimmed text starts with `{` — keeps normal
    // prose from being misread, since a free-form reply can't pass.
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                let args_value = v
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                let args_str =
                    serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string());
                return Some(vec![ToolCallOut {
                    id: format!("call_{}", request_id()),
                    kind: "function",
                    function: ToolCallFunctionOut {
                        name: name.to_string(),
                        arguments: args_str,
                    },
                }]);
            }
        }
    }

    let mut calls = Vec::new();
    let mut cursor = 0;
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        let lt = match text[cursor..].find('<') {
            Some(i) => cursor + i,
            None => break,
        };
        let gt = match text[lt..].find('>') {
            Some(i) => lt + i,
            None => break,
        };
        let tag_inner = &text[lt + 1..gt];
        // Skip closing tags / empty tags / closing on next iteration.
        if tag_inner.starts_with('/') || tag_inner.is_empty() {
            cursor = gt + 1;
            continue;
        }
        // Tag name = leading run of [A-Za-z0-9_-]; ignore attributes.
        let tag_name: String = tag_inner
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if tag_name.is_empty() {
            cursor = gt + 1;
            continue;
        }
        let close = format!("</{tag_name}>");
        let close_idx = match text[gt + 1..].find(close.as_str()) {
            Some(i) => gt + 1 + i,
            None => {
                cursor = gt + 1;
                continue;
            }
        };
        let body = text[gt + 1..close_idx].trim();
        let next_cursor = close_idx + close.len();

        let parsed: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => {
                cursor = next_cursor;
                continue;
            }
        };

        // Pull the tool name. Prefer an explicit `name` field, then fall
        // back to the tag name when the model used `<tool_name>` instead
        // of `<tool_call>`. The literal "tool_call" tag isn't a valid
        // fallback name, so it has to carry an inner `name`.
        let name = parsed
            .get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                if tag_name.eq_ignore_ascii_case("tool_call") {
                    None
                } else {
                    Some(tag_name.clone())
                }
            });
        let name = match name {
            Some(n) => n,
            None => {
                cursor = next_cursor;
                continue;
            }
        };

        // Arguments: prefer an explicit `arguments` field. If the body
        // is an object without one, treat the whole object (minus
        // `name`) as the arguments. Otherwise fall back to `{}`.
        let args_value = if let Some(a) = parsed.get("arguments") {
            a.clone()
        } else if let Some(obj) = parsed.as_object() {
            let mut obj = obj.clone();
            obj.remove("name");
            serde_json::Value::Object(obj)
        } else {
            serde_json::Value::Object(Default::default())
        };
        let args_str = serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string());

        calls.push(ToolCallOut {
            id: format!("call_{}", request_id()),
            kind: "function",
            function: ToolCallFunctionOut {
                name,
                arguments: args_str,
            },
        });
        cursor = next_cursor;
    }
    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
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
mod trim_tests {
    use super::{trim_messages_for_context, ChatRole, CoreChatMessage as Msg};

    fn user(s: &str) -> Msg {
        Msg {
            role: ChatRole::User,
            content: s.to_string(),
        }
    }
    fn assistant(s: &str) -> Msg {
        Msg {
            role: ChatRole::Assistant,
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
    fn passes_through_when_under_budget() {
        let msgs = vec![user("hi"), assistant("hello")];
        let (out, trimmed) = trim_messages_for_context(msgs.clone(), 1000);
        assert!(!trimmed);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn drops_oldest_when_over_budget() {
        // Build a conversation that exceeds budget. With chars/4 + 8 per
        // message, 200-char messages cost ~58 tokens each.
        let big = "x".repeat(200);
        let msgs = vec![
            user(&big),
            assistant(&big),
            user(&big),
            assistant(&big),
            user("now this is the latest"),
        ];
        let (out, trimmed) = trim_messages_for_context(msgs, 100);
        assert!(trimmed);
        // The latest user message must always be kept.
        assert_eq!(out.last().unwrap().content, "now this is the latest");
        // Older turns must have been dropped.
        assert!(out.len() < 5);
    }

    #[test]
    fn preserves_system_message() {
        let big = "x".repeat(200);
        let msgs = vec![
            system("be helpful"),
            user(&big),
            assistant(&big),
            user(&big),
            user("recent"),
        ];
        let (out, trimmed) = trim_messages_for_context(msgs, 100);
        assert!(trimmed);
        assert!(matches!(out[0].role, ChatRole::System));
        assert_eq!(out[0].content, "be helpful");
        assert_eq!(out.last().unwrap().content, "recent");
    }

    #[test]
    fn keeps_latest_user_even_if_over_budget_alone() {
        // A single huge user message over budget should still be kept
        // (otherwise the conversation has no content at all).
        let huge = "y".repeat(8000);
        let msgs = vec![user(&huge)];
        let (out, _) = trim_messages_for_context(msgs, 100);
        assert_eq!(out.len(), 1);
    }
}

#[cfg(test)]
mod tool_tests {
    use super::{
        augment_for_tools, parse_tool_calls, ChatRole, CoreChatMessage as Msg, ToolFunction,
        ToolSpec,
    };

    fn user(s: &str) -> Msg {
        Msg {
            role: ChatRole::User,
            content: s.to_string(),
        }
    }

    fn fn_tool(name: &str, desc: &str) -> ToolSpec {
        ToolSpec {
            kind: "function".to_string(),
            function: ToolFunction {
                name: name.to_string(),
                description: Some(desc.to_string()),
                parameters: Some(serde_json::json!({"type": "object", "properties": {}})),
            },
        }
    }

    #[test]
    fn augment_for_tools_is_noop_when_empty() {
        let msgs = vec![user("hi")];
        let out = augment_for_tools(msgs.clone(), &[]);
        assert_eq!(out.len(), msgs.len());
        assert_eq!(out[0].content, "hi");
    }

    #[test]
    fn augment_for_tools_injects_system_with_tool_names() {
        let tools = vec![
            fn_tool("get_current_time", "current time"),
            fn_tool("calc", "do math"),
        ];
        let out = augment_for_tools(vec![user("hello")], &tools);
        // System message gets prepended.
        assert!(matches!(out[0].role, ChatRole::System));
        assert!(out[0].content.contains("get_current_time"));
        assert!(out[0].content.contains("calc"));
        assert!(out[0].content.contains("<tool_call>"));
        // User message survives at position 1.
        assert_eq!(out[1].content, "hello");
    }

    #[test]
    fn parse_tool_calls_returns_none_when_absent() {
        assert!(parse_tool_calls("hello world").is_none());
    }

    #[test]
    fn parse_tool_calls_extracts_single_call() {
        let text =
            "<tool_call>{\"name\": \"calc\", \"arguments\": {\"expression\": \"2+2\"}}</tool_call>";
        let calls = parse_tool_calls(text).expect("expected one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "calc");
        // Arguments come back as stringified JSON, per OpenAI spec.
        let args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("args parse");
        assert_eq!(args.get("expression").and_then(|v| v.as_str()), Some("2+2"));
    }

    #[test]
    fn parse_tool_calls_skips_malformed_blocks() {
        // First block is junk JSON; second is valid. The good one survives,
        // the bad one is skipped silently.
        let text = "noise <tool_call>not json</tool_call> mid \
                    <tool_call>{\"name\": \"get_current_time\"}</tool_call> end";
        let calls = parse_tool_calls(text).expect("one valid call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_current_time");
        // No `arguments` provided -> empty object string.
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn parse_tool_calls_accepts_tool_name_as_tag() {
        // Phi 3.5 commonly emits `<tool_name>{...}</tool_name>` instead
        // of `<tool_call>` — the parser should still pick it up.
        let text = "<get_current_time>{}</get_current_time>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_current_time");
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn parse_tool_calls_accepts_bare_json() {
        // Phi 3.5 sometimes emits the call object directly with no tag.
        let text = " {\"name\": \"get_current_time\", \"arguments\": {}} ";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_current_time");
        assert_eq!(calls[0].function.arguments, "{}");
    }

    #[test]
    fn parse_tool_calls_accepts_inline_args_under_tag() {
        // Tag name is the tool, inner JSON is just args.
        let text = "<calc>{\"expression\": \"2+2\"}</calc>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "calc");
        let args: serde_json::Value =
            serde_json::from_str(&calls[0].function.arguments).expect("args parse");
        assert_eq!(args.get("expression").and_then(|v| v.as_str()), Some("2+2"));
    }
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

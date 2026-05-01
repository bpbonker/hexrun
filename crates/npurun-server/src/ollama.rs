//! Ollama-compatible HTTP endpoints (`/api/*`).
//!
//! Minimal compatibility surface: enough that Open WebUI and similar
//! Ollama-aware clients see a model and can chat with it. We mirror the
//! NDJSON streaming protocol Ollama uses.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::SystemTime;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
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

/// `/api/*` route subtree.
pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/api/tags", get(api_tags))
        .route("/api/version", get(api_version))
        .route("/api/generate", post(api_generate))
        .route("/api/chat", post(api_chat))
        .route("/api/show", post(api_show))
        .route("/api/delete", post(api_delete))
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
    // Ollama's `/api/tags` returns `<name>:<tag>` rather than a bare
    // name. Open WebUI and the `ollama` CLI rely on the `:latest`
    // suffix when matching what the user typed.
    let models = match state.model_name {
        Some(name) => vec![TagModel {
            name: format!("{name}:latest"),
            modified_at: now_iso8601(),
            size: 0,
            digest: String::new(),
        }],
        None => vec![],
    };
    Json(TagsResponse { models })
}

async fn api_version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize)]
struct ShowRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

async fn api_show(State(state): State<ServerState>, Json(req): Json<ShowRequest>) -> Response {
    let requested = match req.name.or(req.model) {
        Some(n) => n,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "request must include 'name' or 'model'"})),
            )
                .into_response();
        }
    };
    let bare = crate::strip_model_tag(&requested).to_string();

    // Prefer the in-memory engine's manifest when the requested model
    // matches what's loaded — saves a disk read and stays consistent
    // with what the running server is actually serving.
    let manifest = if state.model_name.as_deref() == Some(bare.as_str()) {
        state.engine.as_ref().map(|e| e.manifest().clone())
    } else {
        None
    };

    // Fall back to disk: any cached model under the registry's default
    // cache dir is `/api/show`-able even if it isn't loaded.
    let manifest = manifest.or_else(|| {
        let dir = npurun_registry::default_cache_dir().join(&bare);
        let path = dir.join("npurun.json");
        npurun_core::Manifest::read(&path).ok()
    });

    let manifest = match manifest {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!("model {bare:?} not found in cache"),
                })),
            )
                .into_response();
        }
    };

    // Map our manifest into Ollama's `/api/show` JSON shape. The
    // `details.parameter_size` and `details.quantization_level` fields
    // are the ones Open WebUI surfaces in its model card.
    let template = manifest
        .chat_template
        .as_ref()
        .map(|t| t.template.clone())
        .unwrap_or_default();
    let system = manifest
        .chat_template
        .as_ref()
        .map(|t| t.system_prompt.clone())
        .unwrap_or_default();
    let parameter_size = parameter_size_hint(&manifest.arch, &manifest.name);
    let quant_label = match manifest.quant {
        npurun_core::Quant::Int8 => "INT8",
        npurun_core::Quant::Int8WInt16A => "INT8(W)/INT16(A)",
        npurun_core::Quant::W4A16 => "W4A16",
        npurun_core::Quant::W8A16 => "W8A16",
        npurun_core::Quant::Int4 => "INT4",
        npurun_core::Quant::Fp16 => "FP16",
    };
    Json(serde_json::json!({
        "license": "see npurun and bundle license files",
        "modelfile": format!(
            "# npurun bundle\nFROM {name}\nQNN_SDK {sdk}\nCONTEXT {ctx}\n",
            name = manifest.name,
            sdk = manifest.qnn_sdk,
            ctx = manifest.context,
        ),
        "parameters": format!("context {}", manifest.context),
        "template": template,
        "system": system,
        "details": {
            "format": "qnn-genie-bundle",
            "family": manifest.arch,
            "families": [manifest.arch.clone()],
            "parameter_size": parameter_size,
            "quantization_level": quant_label,
        },
        "model_info": {
            "general.architecture": manifest.arch,
            "general.parameter_count": parameter_size,
            "general.quantization_level": quant_label,
            "qnn.sdk_version": manifest.qnn_sdk,
            "npurun.context": manifest.context,
            "npurun.vocab": manifest.vocab,
        },
    }))
    .into_response()
}

#[derive(Deserialize)]
struct DeleteRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

async fn api_delete(State(state): State<ServerState>, Json(req): Json<DeleteRequest>) -> Response {
    let requested = match req.name.or(req.model) {
        Some(n) => n,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "request must include 'name' or 'model'"})),
            )
                .into_response();
        }
    };
    let bare = crate::strip_model_tag(&requested).to_string();

    // Refuse to remove the model that's currently loaded in this
    // server — the live `Engine` holds open file handles and dropping
    // its on-disk backing store mid-flight would corrupt the running
    // dialog. Clients should restart the server first.
    if state.model_name.as_deref() == Some(bare.as_str()) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!(
                    "{bare:?} is currently loaded by this server; restart npurun serve before deleting"
                ),
            })),
        )
            .into_response();
    }

    match npurun_registry::remove_local(&bare) {
        Ok(removed) => {
            info!(name = %bare, path = %removed.display(), "ollama /api/delete");
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted"})),
            )
                .into_response()
        }
        Err(npurun_registry::RegistryError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("model {bare:?} not found")})),
            )
                .into_response()
        }
        Err(e) => {
            error!(error = %e, name = %bare, "ollama /api/delete failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}

/// Lightweight model-size hint for `/api/show`. Pulled from the bundle
/// name when possible (e.g. "phi-3.5-mini" → "3.8B"; "qwen-2-5-7b" →
/// "7B"). Falls back to "unknown" so we never lie.
fn parameter_size_hint(_arch: &str, name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("phi-3.5-mini") || lower.contains("phi3.5-mini") {
        "3.8B".to_string()
    } else if lower.contains("8b") || lower.contains("-8-") {
        "8B".to_string()
    } else if lower.contains("7b") || lower.contains("-7-") {
        "7B".to_string()
    } else if lower.contains("3b") || lower.contains("-3-") {
        "3B".to_string()
    } else {
        "unknown".to_string()
    }
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
    let permit = match state.inference_permit.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return busy_response(),
    };
    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "npurun".to_string());
    info!(model = %model_name, stream = req.stream, "ollama /api/generate");

    if req.stream {
        let stream = ndjson_stream(engine, permit, req.prompt, model_name);
        ndjson_response(stream).into_response()
    } else {
        match run_blocking(engine, permit, req.prompt).await {
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
    let permit = match state.inference_permit.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return busy_response(),
    };
    let model_name = req
        .model
        .clone()
        .or_else(|| state.model_name.clone())
        .unwrap_or_else(|| "npurun".to_string());
    let messages: Vec<CoreChatMessage> = req
        .messages
        .iter()
        .filter_map(|m| {
            let role = match m.role.as_str() {
                "system" => ChatRole::System,
                "user" => ChatRole::User,
                "assistant" => ChatRole::Assistant,
                _ => return None,
            };
            Some(CoreChatMessage {
                role,
                content: m.content.clone(),
            })
        })
        .collect();
    if !messages.iter().any(|m| m.role == ChatRole::User) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "no user message in request"})),
        )
            .into_response();
    }
    info!(model = %model_name, stream = req.stream, turns = messages.len(), "ollama /api/chat");

    if req.stream {
        let stream = ndjson_chat_stream(engine, permit, messages, model_name);
        ndjson_response(stream).into_response()
    } else {
        match run_blocking_chat(engine, permit, messages).await {
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

fn busy_response() -> Response {
    warn!("inference slot busy, returning 429");
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", "1")],
        Json(serde_json::json!({
            "error": "another inference request is in progress; retry shortly"
        })),
    )
        .into_response()
}

async fn run_blocking(
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    prompt: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        engine.generate(&prompt).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("inference task panicked: {e}"))?
}

async fn run_blocking_chat(
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    messages: Vec<CoreChatMessage>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        engine.generate_chat(&messages).map_err(|e| e.to_string())
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

/// NDJSON stream for `/api/generate` — single-prompt completion. Uses
/// `Engine::generate_streaming` and emits `GenerateChunk`-shaped frames.
/// `/api/chat` uses [`ndjson_chat_stream`] instead.
fn ndjson_stream(
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    user_prompt: String,
    model_name: String,
) -> impl Stream<Item = Result<axum::body::Bytes, Infallible>> {
    let (tx, rx) = mpsc::channel::<StreamItem>(64);
    let tx_clone = tx.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
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
        let bytes = match item {
            StreamItem::Content(s) => {
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
            StreamItem::Done => {
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
            StreamItem::Error(msg) => {
                let payload = serde_json::json!({"error": msg});
                let mut v = serde_json::to_vec(&payload).unwrap_or_default();
                v.push(b'\n');
                v
            }
        };
        Ok(axum::body::Bytes::from(bytes))
    })
}

/// NDJSON stream variant that drives `Engine::generate_chat_streaming`
/// against the full messages array (not just the latest user message).
/// Always emits `ChatChunk` shaped responses — `/api/chat` only.
fn ndjson_chat_stream(
    engine: Arc<Engine>,
    permit: OwnedSemaphorePermit,
    messages: Vec<CoreChatMessage>,
    model_name: String,
) -> impl Stream<Item = Result<axum::body::Bytes, Infallible>> {
    let (tx, rx) = mpsc::channel::<StreamItem>(64);
    let tx_clone = tx.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let res = engine.generate_chat_streaming(&messages, |chunk| {
            let _ = tx_clone.blocking_send(StreamItem::Content(chunk.to_string()));
        });
        match res {
            Ok(()) => {
                let _ = tx_clone.blocking_send(StreamItem::Done);
            }
            Err(e) => {
                error!(error = %e, "ollama streaming chat inference failed");
                let _ = tx_clone.blocking_send(StreamItem::Error(e.to_string()));
            }
        }
    });
    drop(tx);

    ReceiverStream::new(rx).map(move |item| {
        let now = now_iso8601();
        let bytes = match item {
            StreamItem::Content(s) => {
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
            StreamItem::Done => {
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
            StreamItem::Error(msg) => {
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
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u64;
    let (year, month, day) = epoch_days_to_ymd(days);
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

// Howard Hinnant's `days_from_civil` inverse. Converts a count of days
// since the Unix epoch (1970-01-01) into a Gregorian (year, month, day)
// triple. Avoids pulling in `chrono`/`time` for this single call site.
// Reference: http://howardhinnant.github.io/date_algorithms.html
fn epoch_days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d)
}

enum StreamItem {
    Content(String),
    Done,
    Error(String),
}

#[cfg(test)]
mod date_tests {
    use super::epoch_days_to_ymd;

    #[test]
    fn epoch_origin_is_1970_01_01() {
        assert_eq!(epoch_days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn handles_leap_day_2024() {
        // 2024-02-29 is day 19_782 since 1970-01-01.
        assert_eq!(epoch_days_to_ymd(19_782), (2024, 2, 29));
        assert_eq!(epoch_days_to_ymd(19_783), (2024, 3, 1));
    }

    #[test]
    fn handles_year_2026() {
        // 2026-05-01 is day 20_574 since 1970-01-01: 56 full years
        // (1970..2026) at 365 days each (= 20_440) plus 14 leap days
        // (1972, 1976, ..., 2024) gives 20_454 days to 2026-01-01,
        // plus 31+28+31+30 = 120 days into the year.
        assert_eq!(epoch_days_to_ymd(20_574), (2026, 5, 1));
    }

    #[test]
    fn handles_century_non_leap_2100() {
        // 2100 is not a leap year (divisible by 100, not 400).
        // 2100-03-01 is day 47_541 since 1970-01-01.
        assert_eq!(epoch_days_to_ymd(47_540), (2100, 2, 28));
        assert_eq!(epoch_days_to_ymd(47_541), (2100, 3, 1));
    }
}

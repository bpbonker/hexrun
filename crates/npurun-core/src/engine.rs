//! Inference engine: load a model bundle, run a generation loop.
//!
//! When the `genie` feature is enabled, [`Engine`] holds a `qnn::Dialog`
//! and drives the Hexagon NPU via Qualcomm's Genie LLM runtime. Without
//! the feature, `Engine::generate` returns [`EngineError::FeatureDisabled`].

use std::path::PathBuf;
#[cfg(feature = "genie")]
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
#[cfg(feature = "genie")]
use tracing::debug;
use tracing::info;

use crate::manifest::{ChatMessage, Manifest, ManifestError};

/// Inference backend selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    /// Default for LLMs: Qualcomm Genie LLM runtime.
    #[default]
    Genie,
    /// ONNX Runtime via the QNN Execution Provider. For non-LLM models.
    Ort,
    /// CPU fallback. Useful for smoke tests when the NPU is unavailable.
    Cpu,
}

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Directory containing `npurun.json` and the files it references.
    pub model_dir: PathBuf,
    /// Backend to use.
    pub backend: Backend,
    /// Maximum tokens to generate per call. Currently advisory — Genie
    /// honours its own internal generation limits via the model config.
    pub max_tokens: usize,
    /// Override for the Genie dialog's context size (the `dialog.context.size`
    /// field of `genie_config.json`). When `Some(n)`, the engine pins the
    /// dialog to that tier, validating `n` against the compiled
    /// `clNNNN` tiers shipped in the bundle's `ctx-bins`. `None` (the
    /// default) lets the bundle use whatever the manifest declared.
    pub ctx: Option<u32>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::new(),
            backend: Backend::Genie,
            max_tokens: 512,
            ctx: None,
        }
    }
}

/// Errors raised by the engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Model directory does not exist.
    #[error("model directory does not exist: {0}")]
    ModelDirMissing(PathBuf),
    /// Manifest could not be loaded.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Manifest does not declare a Genie bundle (`files.genie_config`).
    #[error("manifest at {path} has no files.genie_config; cannot run on Genie backend")]
    NotAGenieBundle {
        /// Path of the manifest.
        path: PathBuf,
    },
    /// The compiled-in feature for the requested backend is disabled.
    #[error("backend {0:?} is not available — rebuild npurun-core with the appropriate feature")]
    FeatureDisabled(Backend),
    /// An error from the Genie C runtime.
    #[cfg(feature = "genie")]
    #[error(transparent)]
    Genie(#[from] qnn::GenieError),
    /// A chat request had no user message in it.
    #[error("chat request contained no user message")]
    NoUserMessage,
    /// A chat request was sent to a model whose manifest doesn't
    /// declare a chat template.
    #[error("model {name} has no chat_template; cannot run multi-turn chat")]
    NoChatTemplate {
        /// Manifest name of the model.
        name: String,
    },
    /// Reading or parsing the bundle's `genie_config.json` failed while
    /// applying a context-size override.
    #[error("failed to read genie_config.json at {path}: {message}")]
    GenieConfigRead {
        /// Path of the genie_config.json that failed to load.
        path: PathBuf,
        /// Human-readable detail.
        message: String,
    },
    /// The user requested a context tier the bundle wasn't compiled with.
    #[error(
        "context tier {requested} not available in this bundle; available tiers: {}",
        format_tiers(.available)
    )]
    ContextTierUnavailable {
        /// Requested tier (the `--ctx` value).
        requested: u32,
        /// Tiers actually compiled into the bundle, ascending.
        available: Vec<u32>,
    },
}

fn format_tiers(tiers: &[u32]) -> String {
    if tiers.is_empty() {
        "(none — could not parse ctx-bins filenames)".to_string()
    } else {
        tiers
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Extract the compiled context tiers from a parsed `genie_config.json`.
///
/// The Genie bundle layout shipped by Qualcomm encodes the tiers in the
/// `ctx-bins` shard filenames as `clNNNN` tokens (e.g.
/// `weight_sharing_model_ar128_ar1_cl512_cl1024_cl2048_cl3072_cl4096_1_of_4.serialized.bin`).
/// We deduplicate, sort ascending, and return the result. Returns an
/// empty vector if the JSON has no `dialog.engine.model.binary.ctx-bins`
/// array (caller decides whether to treat that as "no validation
/// possible" or as a hard error).
fn available_ctx_tiers(json: &serde_json::Value) -> Vec<u32> {
    let Some(bins) = json
        .get("dialog")
        .and_then(|d| d.get("engine"))
        .and_then(|e| e.get("model"))
        .and_then(|m| m.get("binary"))
        .and_then(|b| b.get("ctx-bins"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    let mut tiers: Vec<u32> = Vec::new();
    for bin in bins {
        let Some(name) = bin.as_str() else { continue };
        for (i, _) in name.match_indices("cl") {
            let rest = &name[i + 2..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<u32>() {
                if !tiers.contains(&n) {
                    tiers.push(n);
                }
            }
        }
    }
    tiers.sort_unstable();
    tiers
}

/// Loaded inference engine.
pub struct Engine {
    manifest: Manifest,
    config: EngineConfig,
    #[cfg(feature = "genie")]
    dialog: qnn::Dialog,
    /// Tracks whether the dialog's KV cache has been primed by at least
    /// one chat call. Genie rejects `SentenceCode::Rewind` (with
    /// `ERROR_QUERY_FAILED`) when the cache is empty — the first turn
    /// of a fresh dialog must use `Begin`. After that, every subsequent
    /// call uses `Rewind` so Genie can prefix-match the transcript and
    /// only re-prefill the new tokens.
    #[cfg(feature = "genie")]
    chat_started: AtomicBool,
}

impl Engine {
    /// Load a model from a directory containing `npurun.json`.
    ///
    /// On the Genie backend, also opens the bundle's `genie_config.json`
    /// and creates the Genie dialog (the dialog stays resident in NPU
    /// shared memory until [`Engine`] is dropped). Returns a plain
    /// `Engine` so callers can wrap it in whatever sharing primitive
    /// fits their use case (`Arc<Mutex<Engine>>` for the HTTP server,
    /// just `Engine` for the CLI's one-shot run).
    pub fn load(config: EngineConfig) -> Result<Self, EngineError> {
        if !config.model_dir.is_dir() {
            return Err(EngineError::ModelDirMissing(config.model_dir.clone()));
        }
        let manifest_path = config.model_dir.join("npurun.json");
        let manifest = Manifest::read(&manifest_path)?;
        info!(
            name = %manifest.name,
            arch = %manifest.arch,
            quant = ?manifest.quant,
            qnn_sdk = %manifest.qnn_sdk,
            backend = ?config.backend,
            "loaded manifest"
        );

        match config.backend {
            Backend::Genie => Self::load_genie(manifest, config),
            other => Err(EngineError::FeatureDisabled(other)),
        }
    }

    #[cfg(feature = "genie")]
    fn load_genie(manifest: Manifest, config: EngineConfig) -> Result<Self, EngineError> {
        let genie_rel =
            manifest
                .files
                .genie_config
                .as_ref()
                .ok_or_else(|| EngineError::NotAGenieBundle {
                    path: config.model_dir.join("npurun.json"),
                })?;
        let genie_path = config.model_dir.join(genie_rel);
        debug!(path = %genie_path.display(), "opening Genie config");

        let dialog = match config.ctx {
            None => qnn::Dialog::from_config_file(&genie_path)?,
            Some(requested) => {
                let raw = std::fs::read_to_string(&genie_path).map_err(|e| {
                    EngineError::GenieConfigRead {
                        path: genie_path.clone(),
                        message: e.to_string(),
                    }
                })?;
                let mut json: serde_json::Value =
                    serde_json::from_str(&raw).map_err(|e| EngineError::GenieConfigRead {
                        path: genie_path.clone(),
                        message: format!("invalid JSON: {e}"),
                    })?;
                let available = available_ctx_tiers(&json);
                if !available.contains(&requested) {
                    return Err(EngineError::ContextTierUnavailable {
                        requested,
                        available,
                    });
                }
                if let Some(size) = json
                    .get_mut("dialog")
                    .and_then(|d| d.get_mut("context"))
                    .and_then(|c| c.get_mut("size"))
                {
                    *size = serde_json::Value::from(requested);
                } else {
                    return Err(EngineError::GenieConfigRead {
                        path: genie_path.clone(),
                        message: "missing dialog.context.size; cannot apply --ctx override"
                            .to_string(),
                    });
                }
                let parent = genie_path
                    .parent()
                    .ok_or_else(|| EngineError::GenieConfigRead {
                        path: genie_path.clone(),
                        message: "config path has no parent directory".to_string(),
                    })?;
                let patched =
                    serde_json::to_string(&json).map_err(|e| EngineError::GenieConfigRead {
                        path: genie_path.clone(),
                        message: format!("re-serializing patched config: {e}"),
                    })?;
                info!(requested, "pinning Genie context tier via --ctx");
                qnn::Dialog::from_config_json_in_dir(&patched, parent)?
            }
        };
        Ok(Self {
            manifest,
            config,
            dialog,
            chat_started: AtomicBool::new(false),
        })
    }

    /// Tiers (in ascending order) that this engine's bundle was compiled
    /// for, parsed from the `ctx-bins` filenames the way Genie itself
    /// names them (`...cl512_cl1024_cl2048_..._N_of_M.serialized.bin`).
    /// Useful for surfacing valid `--ctx` choices without re-loading the
    /// bundle. Returns an empty vector if the bundle's config could not
    /// be read (e.g. when the engine wasn't loaded from disk).
    pub fn available_ctx_tiers(model_dir: &std::path::Path) -> Vec<u32> {
        let manifest_path = model_dir.join("npurun.json");
        let Ok(manifest) = Manifest::read(&manifest_path) else {
            return Vec::new();
        };
        let Some(genie_rel) = manifest.files.genie_config.as_ref() else {
            return Vec::new();
        };
        let genie_path = model_dir.join(genie_rel);
        let Ok(raw) = std::fs::read_to_string(&genie_path) else {
            return Vec::new();
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Vec::new();
        };
        available_ctx_tiers(&json)
    }

    #[cfg(not(feature = "genie"))]
    #[allow(clippy::needless_pass_by_value)]
    fn load_genie(_manifest: Manifest, _config: EngineConfig) -> Result<Self, EngineError> {
        Err(EngineError::FeatureDisabled(Backend::Genie))
    }

    /// Manifest of the loaded model.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Resolved engine config.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Run a single, blocking query and return the full generated response.
    ///
    /// The user prompt is wrapped using the manifest's `chat_template`
    /// (if present); otherwise it's passed through unchanged.
    #[cfg(feature = "genie")]
    pub fn generate(&self, prompt: &str) -> Result<String, EngineError> {
        let wrapped = match &self.manifest.chat_template {
            Some(t) => t.wrap(prompt),
            None => prompt.to_string(),
        };
        Ok(self.dialog.query(&wrapped)?)
    }

    /// Run a query and invoke `callback` for each response chunk.
    #[cfg(feature = "genie")]
    pub fn generate_streaming<F>(&self, prompt: &str, mut callback: F) -> Result<(), EngineError>
    where
        F: FnMut(&str),
    {
        let wrapped = match &self.manifest.chat_template {
            Some(t) => t.wrap(prompt),
            None => prompt.to_string(),
        };
        self.dialog.query_streaming(&wrapped, |chunk, _code| {
            callback(chunk);
        })?;
        Ok(())
    }

    /// Run a multi-turn chat query against a full message history and
    /// invoke `callback` for each response chunk.
    ///
    /// Builds the transcript via the manifest's chat template, then
    /// sends it to Genie with [`qnn::SentenceCode::Rewind`]. On the
    /// first call the dialog's KV cache is empty, so Rewind degrades
    /// to a fresh prefill. On subsequent calls Genie matches the
    /// transcript prefix against what's already in the cache and
    /// re-prefills only the new tokens — which on a typical chat is
    /// just the latest assistant reply (already in cache from the
    /// previous turn) plus the new user message.
    ///
    /// Returns `EngineError::NoChatTemplate` if the manifest has no
    /// `chat_template`, or `EngineError::NoUserMessage` if `messages`
    /// has no user turn.
    #[cfg(feature = "genie")]
    pub fn generate_chat_streaming<F>(
        &self,
        messages: &[ChatMessage],
        mut callback: F,
    ) -> Result<(), EngineError>
    where
        F: FnMut(&str),
    {
        let template =
            self.manifest
                .chat_template
                .as_ref()
                .ok_or_else(|| EngineError::NoChatTemplate {
                    name: self.manifest.name.clone(),
                })?;
        let transcript = template
            .wrap_chat(messages)
            .ok_or(EngineError::NoUserMessage)?;
        // Sentence-code selection for multi-turn:
        //
        // - First call on a freshly loaded dialog: `Complete`. This is
        //   a self-contained, single-shot query; Genie processes the
        //   whole transcript and leaves the resulting KV cache state
        //   resident on the dialog.
        // - Subsequent calls: `Rewind`. Genie matches the new
        //   transcript prefix against the cached state, rewinds the
        //   KV cache to the divergence point, and re-prefills only
        //   the new tokens — the multi-turn fast path.
        //
        // (`Begin`/`Continue`/`End` are for chunked *input* — feeding
        // one long prompt to Genie in pieces. Not what we want here.)
        //
        // Compare-and-swap so concurrent first calls — if they ever
        // slipped past the server's semaphore — would still pick
        // exactly one `Complete`.
        let code = if self
            .chat_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            qnn::SentenceCode::Complete
        } else {
            qnn::SentenceCode::Rewind
        };
        self.dialog
            .query_streaming_with(&transcript, code, |chunk, _code| {
                callback(chunk);
            })?;
        Ok(())
    }

    /// Drop the underlying Genie dialog's state. Clears the KV cache and
    /// forces the next [`Self::generate_chat_streaming`] call to use
    /// `SentenceCode::Begin`. Call this whenever you want guaranteed-clean
    /// state instead of relying on Genie's prefix-mismatch handling — when
    /// a chat client signals a fresh conversation, or between independent
    /// single-shot [`Self::generate_streaming`] calls in benchmark loops
    /// (otherwise the prior turn's tokens stay in cache, the next
    /// generation runs in a contaminated context, and Genie eventually
    /// returns ERROR_QUERY_FAILED).
    #[cfg(feature = "genie")]
    pub fn reset_dialog(&self) -> Result<(), EngineError> {
        self.dialog.reset()?;
        self.chat_started.store(false, Ordering::Release);
        Ok(())
    }

    /// Signal an in-flight query to abort. Intended to be called from a
    /// different thread than the one blocked inside Genie — typically from
    /// an HTTP handler that detects the client has disconnected, so the
    /// blocking inference task on the Tokio blocking pool can wind down
    /// and release its inference permit instead of running to completion.
    ///
    /// Genie checks for the signal between generated tokens and returns
    /// from `query_streaming*` with `SentenceCode::Abort`. The caller
    /// should then [`Self::reset_dialog`] before issuing another query.
    #[cfg(feature = "genie")]
    pub fn signal_abort(&self) -> Result<(), EngineError> {
        self.dialog.signal_abort()?;
        Ok(())
    }

    /// Blocking variant of [`Self::generate_chat_streaming`].
    #[cfg(feature = "genie")]
    pub fn generate_chat(&self, messages: &[ChatMessage]) -> Result<String, EngineError> {
        let mut buf = String::new();
        self.generate_chat_streaming(messages, |chunk| buf.push_str(chunk))?;
        Ok(buf)
    }

    /// Stub implementation when the `genie` feature is off.
    #[cfg(not(feature = "genie"))]
    pub fn generate(&self, _prompt: &str) -> Result<String, EngineError> {
        Err(EngineError::FeatureDisabled(self.config.backend))
    }

    /// Stub implementation when the `genie` feature is off.
    #[cfg(not(feature = "genie"))]
    pub fn generate_streaming<F>(&self, _prompt: &str, _callback: F) -> Result<(), EngineError>
    where
        F: FnMut(&str),
    {
        Err(EngineError::FeatureDisabled(self.config.backend))
    }

    /// Stub implementation when the `genie` feature is off.
    #[cfg(not(feature = "genie"))]
    pub fn generate_chat(&self, _messages: &[ChatMessage]) -> Result<String, EngineError> {
        Err(EngineError::FeatureDisabled(self.config.backend))
    }

    /// Stub implementation when the `genie` feature is off.
    #[cfg(not(feature = "genie"))]
    pub fn generate_chat_streaming<F>(
        &self,
        _messages: &[ChatMessage],
        _callback: F,
    ) -> Result<(), EngineError>
    where
        F: FnMut(&str),
    {
        Err(EngineError::FeatureDisabled(self.config.backend))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_rejects_missing_dir() {
        let cfg = EngineConfig {
            model_dir: PathBuf::from("definitely/does/not/exist/abcxyz"),
            ..Default::default()
        };
        match Engine::load(cfg) {
            Err(EngineError::ModelDirMissing(_)) => {}
            Err(other) => panic!("unexpected error variant: {other:?}"),
            Ok(_) => panic!("expected an error from missing model_dir"),
        }
    }

    #[test]
    fn defaults_use_genie_backend() {
        assert_eq!(EngineConfig::default().backend, Backend::Genie);
    }

    #[test]
    fn parses_phi_ctx_tiers_from_genie_config() {
        let json = serde_json::json!({
            "dialog": {
                "engine": {
                    "model": {
                        "binary": {
                            "ctx-bins": [
                                "weight_sharing_model_ar128_ar1_cl512_cl1024_cl2048_cl3072_cl4096_1_of_4.serialized.bin",
                                "weight_sharing_model_ar128_ar1_cl512_cl1024_cl2048_cl3072_cl4096_2_of_4.serialized.bin"
                            ]
                        }
                    }
                }
            }
        });
        assert_eq!(
            available_ctx_tiers(&json),
            vec![512, 1024, 2048, 3072, 4096]
        );
    }

    #[test]
    fn empty_when_genie_config_lacks_ctx_bins() {
        let json = serde_json::json!({"dialog": {}});
        assert_eq!(available_ctx_tiers(&json), Vec::<u32>::new());
    }

    #[test]
    fn defaults_use_no_ctx_override() {
        assert_eq!(EngineConfig::default().ctx, None);
    }
}

//! `npurun.json` model manifest format.
//!
//! Every model in the registry ships with a manifest. The manifest is the
//! integrity boundary: file references, sha256s, quant scheme, and the QNN
//! SDK version against which the context binary was compiled.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

/// Quantization schemes npurun understands. The HTP backend on Snapdragon X
/// requires one of these — full FP32 is not supported on the NPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quant {
    /// Per-tensor INT8 weights and activations.
    Int8,
    /// Per-channel INT8 weights with INT16 activations (recommended for
    /// attention stability on most LLMs).
    #[serde(rename = "int8-w-int16-a")]
    Int8WInt16A,
    /// 4-bit weights with 16-bit activations (Qualcomm's standard for
    /// pre-built Snapdragon X Elite LLM bundles).
    #[serde(rename = "w4a16")]
    W4A16,
    /// 8-bit weights with 16-bit activations.
    #[serde(rename = "w8a16")]
    W8A16,
    /// 4-bit weights (group-quantized). Smallest footprint; quality varies
    /// by model.
    Int4,
    /// FP16 — supported on GPU/CPU EPs but generally not on HTP.
    Fp16,
}

/// A `npurun.json` manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Registry name, e.g. `"phi-3.5-mini"`.
    pub name: String,
    /// Manifest version (model release), e.g. `"1.0.0"`.
    pub version: String,
    /// Model architecture identifier (`"phi3"`, `"llama"`, `"qwen2"`, ...).
    pub arch: String,
    /// Vocabulary size of the tokenizer.
    pub vocab: u32,
    /// Maximum context length (positions) the model was built for.
    pub context: u32,
    /// Quantization scheme used to produce the model.
    pub quant: Quant,
    /// QNN SDK version against which the context binary was compiled, e.g.
    /// "2.44.0". Used by the runtime to refuse loading on a too-different
    /// runtime — see [`Manifest::check_sdk_compat`].
    pub qnn_sdk: String,
    /// Files that ship with this model.
    pub files: ManifestFiles,
    /// Chat-template wrapping for user prompts. Different model families
    /// (Phi 3 vs. Qwen 2.5 vs. Llama 3) use different wrapping syntaxes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template: Option<ChatTemplate>,
    /// sha256 hex digests for each file referenced under [`Self::files`].
    /// Keyed by the field name (e.g. `"model"`, `"ctx"`, `"tokenizer"`).
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
}

/// Files referenced by a manifest. All paths are relative to the directory
/// containing `npurun.json`. Absolute paths and `..` segments are rejected
/// at validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestFiles {
    /// ONNX file (path relative to the manifest's directory). Used for
    /// non-Genie inference paths. Optional — Genie bundles don't need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional QNN context binary (`*.qnn_ctx.bin`) for fast cold start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx: Option<String>,
    /// HuggingFace-style `tokenizer.json` file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
    /// Optional architecture/runtime config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// Path to a Genie `genie_config.json`. When present, npurun loads the
    /// model via the Genie LLM runtime (the LLM-NPU path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genie_config: Option<String>,
}

/// Chat-template configuration. Drives both single-turn wrapping (the
/// CLI's `npurun run` and `npurun bench`) and multi-turn transcript
/// construction (the HTTP server's `/v1/chat/completions` and
/// `/api/chat` endpoints).
///
/// For single-turn wrapping the [`Self::wrap`] method substitutes
/// `{system}` and `{user}` into [`Self::template`]. For multi-turn
/// transcripts, [`Self::wrap_chat`] additionally uses
/// [`Self::assistant_turn`] (formats a previous assistant reply) and
/// [`Self::next_user_turn`] (opens a new user turn). The two extra
/// fields are optional; without them, `wrap_chat` falls back to the
/// last user message wrapped via [`Self::template`] (turn-1 only — fine
/// for single-turn API requests).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTemplate {
    /// Default system prompt (substituted into `{system}` in `template`).
    pub system_prompt: String,
    /// Format string for the first turn (system + first user message).
    /// `{system}` and `{user}` placeholders. Must end at the
    /// "assistant turn starts here" marker so generation begins with
    /// the assistant's response.
    pub template: String,
    /// Optional format string for an assistant message in the
    /// transcript, with a single `{assistant}` placeholder. Required
    /// for multi-turn chat. Example for Phi 3:
    /// `"{assistant}<|end|>\n"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_turn: Option<String>,
    /// Optional format string for a follow-up user turn, with a single
    /// `{user}` placeholder. Must terminate at the "assistant turn
    /// starts here" marker. Required for multi-turn chat. Example for
    /// Phi 3: `"<|user|>\n{user}<|end|>\n<|assistant|>\n"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_user_turn: Option<String>,
}

/// Role of a chat message. Mirrors the OpenAI / Ollama API enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    /// System / instruction message. Only the first such message is
    /// honoured; the rest are merged into the conversation as if user.
    System,
    /// A user (human) turn.
    User,
    /// An assistant (model) turn.
    Assistant,
}

/// A single chat message. Lightweight, owned, suitable for crossing the
/// HTTP-handler / engine boundary without lifetime gymnastics.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// The role of the speaker.
    pub role: ChatRole,
    /// Message content (plain text — multimodal is out of scope).
    pub content: String,
}

impl ChatTemplate {
    /// Wrap a single user prompt using the first-turn template.
    /// `{system}` is replaced with [`Self::system_prompt`]; `{user}`
    /// with the supplied prompt.
    pub fn wrap(&self, user: &str) -> String {
        self.template
            .replace("{system}", &self.system_prompt)
            .replace("{user}", user)
    }

    /// Build a full multi-turn transcript from a sequence of chat
    /// messages.
    ///
    /// Behaviour:
    ///
    /// 1. The first `system` message in `messages` (if any) overrides
    ///    [`Self::system_prompt`]. Other system messages are ignored.
    /// 2. The first user message is rendered through [`Self::template`]
    ///    along with the chosen system prompt.
    /// 3. Each subsequent assistant message is rendered through
    ///    [`Self::assistant_turn`].
    /// 4. Each subsequent user message is rendered through
    ///    [`Self::next_user_turn`].
    /// 5. The transcript ends with the assistant marker open — i.e.
    ///    Genie generates the next assistant turn.
    ///
    /// If [`Self::assistant_turn`] or [`Self::next_user_turn`] is not
    /// set on this template, multi-turn rendering degrades to using
    /// only the most recent user message via [`Self::template`]. This
    /// keeps single-turn bundles working while older bundles without
    /// the extra fields are still readable.
    ///
    /// Returns `None` if there is no user message at all (caller should
    /// reject the request as malformed).
    pub fn wrap_chat(&self, messages: &[ChatMessage]) -> Option<String> {
        // Pick the system prompt: explicit `system` message wins, else
        // the template's default.
        let system = messages
            .iter()
            .find(|m| m.role == ChatRole::System)
            .map(|m| m.content.as_str())
            .unwrap_or(&self.system_prompt);

        // Filter out system messages — we've already extracted the one
        // that matters.
        let turns: Vec<&ChatMessage> = messages
            .iter()
            .filter(|m| m.role != ChatRole::System)
            .collect();

        if turns.is_empty() {
            return None;
        }

        // Multi-turn requires both extra fields. Fall back to single-turn
        // when either is missing: just wrap the most recent user message
        // through `template`. (This is the same behaviour as the
        // pre-multi-turn server; preserves backward compat.)
        let (Some(asst_t), Some(user_t)) = (&self.assistant_turn, &self.next_user_turn) else {
            let last_user = turns
                .iter()
                .rev()
                .find(|m| m.role == ChatRole::User)
                .map(|m| m.content.as_str())?;
            let wrapped = self
                .template
                .replace("{system}", system)
                .replace("{user}", last_user);
            return Some(wrapped);
        };

        // Walk the turns in order. The first user turn opens via
        // `template` (which carries the system block); subsequent user
        // turns open via `next_user_turn`; assistant turns close via
        // `assistant_turn`.
        let mut out = String::new();
        let mut first_user_emitted = false;
        for m in turns {
            match m.role {
                ChatRole::User => {
                    if !first_user_emitted {
                        out.push_str(
                            &self
                                .template
                                .replace("{system}", system)
                                .replace("{user}", &m.content),
                        );
                        first_user_emitted = true;
                    } else {
                        out.push_str(&user_t.replace("{user}", &m.content));
                    }
                }
                ChatRole::Assistant => {
                    // An assistant turn that arrives before the first
                    // user turn doesn't really make sense — the OpenAI
                    // / Ollama wire format expects a user message to
                    // open the conversation. Skip it; ill-formed
                    // requests degrade to the same behaviour as if
                    // the assistant turn weren't there.
                    if !first_user_emitted {
                        continue;
                    }
                    out.push_str(&asst_t.replace("{assistant}", &m.content));
                }
                ChatRole::System => unreachable!("system filtered above"),
            }
        }

        if !first_user_emitted {
            return None;
        }
        Some(out)
    }
}

/// Errors raised while reading or validating a manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// Reading the manifest file from disk failed.
    #[error("manifest at {path}: {source}")]
    Io {
        /// Path of the manifest file.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Manifest is not valid JSON or doesn't match the expected schema.
    #[error("manifest at {path} is not valid JSON: {source}")]
    Parse {
        /// Path of the manifest file.
        path: PathBuf,
        /// Underlying JSON parse error.
        #[source]
        source: serde_json::Error,
    },
    /// Manifest fields were structurally valid but failed semantic validation.
    #[error("manifest validation failed: {0}")]
    Invalid(String),
    /// The runtime QNN SDK is too different from the manifest's
    /// `qnn_sdk` field — major-version mismatch.
    #[error(
        "manifest QNN SDK version {manifest} differs from runtime {runtime} by major; \
         refusing to load. Re-pull or re-convert the model."
    )]
    SdkMajorMismatch {
        /// The version recorded in the manifest.
        manifest: String,
        /// The live runtime version.
        runtime: String,
    },
}

impl Manifest {
    /// Read and validate a manifest from disk.
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        let bytes = fs::read(path).map_err(|e| ManifestError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let m: Self = serde_json::from_slice(&bytes).map_err(|e| ManifestError::Parse {
            path: path.to_path_buf(),
            source: e,
        })?;
        m.validate()?;
        Ok(m)
    }

    /// Validate field invariants. Called by `read`; also useful in tests.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty() {
            return Err(ManifestError::Invalid("name is empty".into()));
        }
        if self.version.is_empty() {
            return Err(ManifestError::Invalid("version is empty".into()));
        }
        if self.vocab == 0 {
            return Err(ManifestError::Invalid("vocab is 0".into()));
        }
        if self.context == 0 {
            return Err(ManifestError::Invalid("context is 0".into()));
        }
        if !is_semverish(&self.qnn_sdk) {
            return Err(ManifestError::Invalid(format!(
                "qnn_sdk {:?} is not in MAJOR.MINOR.PATCH form",
                self.qnn_sdk
            )));
        }
        if let Some(ref m) = self.files.model {
            check_safe_relpath("files.model", m)?;
        }
        if let Some(ref t) = self.files.tokenizer {
            check_safe_relpath("files.tokenizer", t)?;
        }
        if let Some(ref ctx) = self.files.ctx {
            check_safe_relpath("files.ctx", ctx)?;
        }
        if let Some(ref cfg) = self.files.config {
            check_safe_relpath("files.config", cfg)?;
        }
        if let Some(ref gc) = self.files.genie_config {
            check_safe_relpath("files.genie_config", gc)?;
        }
        if self.files.model.is_none() && self.files.genie_config.is_none() {
            return Err(ManifestError::Invalid(
                "manifest must specify either files.model (ORT path) \
                 or files.genie_config (Genie path)"
                    .into(),
            ));
        }
        for (key, hex) in &self.sha256 {
            if !is_sha256_hex(hex) {
                return Err(ManifestError::Invalid(format!(
                    "sha256[{key}] is not 64 lowercase hex chars"
                )));
            }
        }
        if let Some(ref t) = self.chat_template {
            if !t.template.contains("{user}") {
                return Err(ManifestError::Invalid(
                    "chat_template.template must contain {user} placeholder".into(),
                ));
            }
        }
        Ok(())
    }

    /// Compare this manifest's `qnn_sdk` against a runtime version. Returns
    /// Ok if compatible (with a warning logged for minor drift), Err on
    /// major drift. Patch drift is silent.
    pub fn check_sdk_compat(&self, runtime_sdk: &str) -> Result<(), ManifestError> {
        let m = parse_semver(&self.qnn_sdk).ok_or_else(|| {
            ManifestError::Invalid(format!("qnn_sdk {:?} unparseable", self.qnn_sdk))
        })?;
        let r = match parse_semver(runtime_sdk) {
            Some(v) => v,
            None => {
                warn!(
                    runtime = runtime_sdk,
                    "runtime QNN version unparseable; skipping check"
                );
                return Ok(());
            }
        };
        if m.0 != r.0 {
            return Err(ManifestError::SdkMajorMismatch {
                manifest: self.qnn_sdk.clone(),
                runtime: runtime_sdk.to_string(),
            });
        }
        if m.1 != r.1 {
            warn!(
                manifest = %self.qnn_sdk,
                runtime = %runtime_sdk,
                "QNN SDK minor version mismatch; context binary may need recompile"
            );
        }
        Ok(())
    }
}

fn check_safe_relpath(field: &str, p: &str) -> Result<(), ManifestError> {
    if p.is_empty() {
        return Err(ManifestError::Invalid(format!("{field} is empty")));
    }
    let path = Path::new(p);
    if path.is_absolute() {
        return Err(ManifestError::Invalid(format!("{field} {p:?} is absolute")));
    }
    for c in path.components() {
        match c {
            Component::ParentDir => {
                return Err(ManifestError::Invalid(format!(
                    "{field} {p:?} contains '..'"
                )));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ManifestError::Invalid(format!(
                    "{field} {p:?} contains a root or drive prefix"
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_semverish(s: &str) -> bool {
    parse_semver(s).is_some()
}

fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    // Strip any "-pre" or "+build" suffix first so MAJOR.MINOR.PATCH parsing
    // is straightforward, including for inputs like "2.44.0+build.5".
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Manifest {
        Manifest {
            name: "phi-3.5-mini".into(),
            version: "1.0.0".into(),
            arch: "phi3".into(),
            vocab: 32064,
            context: 4096,
            quant: Quant::W4A16,
            qnn_sdk: "2.45.0".into(),
            files: ManifestFiles {
                model: None,
                ctx: None,
                tokenizer: Some("tokenizer.json".into()),
                config: None,
                genie_config: Some("bundle/genie_config.json".into()),
            },
            chat_template: Some(ChatTemplate {
                system_prompt: "You are a helpful assistant.".into(),
                template: "<|system|>\n{system}<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n"
                    .into(),
                assistant_turn: Some("{assistant}<|end|>\n".into()),
                next_user_turn: Some("<|user|>\n{user}<|end|>\n<|assistant|>\n".into()),
            }),
            sha256: BTreeMap::new(),
        }
    }

    #[test]
    fn roundtrip_json() {
        let m = good();
        let s = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&s).unwrap();
        back.validate().unwrap();
        assert_eq!(back.name, m.name);
        assert_eq!(back.quant, Quant::W4A16);
    }

    #[test]
    fn rejects_empty_name() {
        let mut m = good();
        m.name.clear();
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_zero_vocab() {
        let mut m = good();
        m.vocab = 0;
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_path_traversal() {
        let mut m = good();
        m.files.tokenizer = Some("../etc/passwd".into());
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(s) if s.contains("'..'")));
    }

    #[test]
    fn rejects_absolute_path() {
        let mut m = good();
        m.files.genie_config = Some("/abs/genie.json".into());
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_drive_prefix() {
        let mut m = good();
        m.files.genie_config = Some("C:\\abs\\genie.json".into());
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_bad_sha256() {
        let mut m = good();
        m.sha256.insert("model".into(), "not-a-real-hex".into());
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn accepts_valid_sha256() {
        let mut m = good();
        m.sha256.insert("model".into(), "a".repeat(64));
        m.validate().unwrap();
    }

    #[test]
    fn rejects_bad_qnn_sdk_version() {
        let mut m = good();
        m.qnn_sdk = "two.point.four".into();
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_neither_model_nor_genie_config() {
        let mut m = good();
        m.files.genie_config = None;
        m.files.model = None;
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_chat_template_without_user_placeholder() {
        let mut m = good();
        m.chat_template = Some(ChatTemplate {
            system_prompt: "x".into(),
            template: "no placeholder".into(),
            assistant_turn: None,
            next_user_turn: None,
        });
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn chat_template_wrap_substitutes_placeholders() {
        let t = ChatTemplate {
            system_prompt: "be helpful".into(),
            template: "[S]{system}[U]{user}[A]".into(),
            assistant_turn: None,
            next_user_turn: None,
        };
        assert_eq!(t.wrap("hi"), "[S]be helpful[U]hi[A]");
    }

    fn multi_turn_template() -> ChatTemplate {
        ChatTemplate {
            system_prompt: "default-system".into(),
            template: "[S]{system}[U]{user}[A]".into(),
            assistant_turn: Some("{assistant}[/A]".into()),
            next_user_turn: Some("[U]{user}[A]".into()),
        }
    }

    #[test]
    fn wrap_chat_single_user_turn() {
        let t = multi_turn_template();
        let msgs = vec![ChatMessage {
            role: ChatRole::User,
            content: "hello".into(),
        }];
        assert_eq!(t.wrap_chat(&msgs).unwrap(), "[S]default-system[U]hello[A]");
    }

    #[test]
    fn wrap_chat_multi_turn() {
        let t = multi_turn_template();
        let msgs = vec![
            ChatMessage { role: ChatRole::User, content: "u1".into() },
            ChatMessage { role: ChatRole::Assistant, content: "a1".into() },
            ChatMessage { role: ChatRole::User, content: "u2".into() },
        ];
        assert_eq!(
            t.wrap_chat(&msgs).unwrap(),
            "[S]default-system[U]u1[A]a1[/A][U]u2[A]"
        );
    }

    #[test]
    fn wrap_chat_explicit_system_overrides_default() {
        let t = multi_turn_template();
        let msgs = vec![
            ChatMessage { role: ChatRole::System, content: "be terse".into() },
            ChatMessage { role: ChatRole::User, content: "hi".into() },
        ];
        assert_eq!(t.wrap_chat(&msgs).unwrap(), "[S]be terse[U]hi[A]");
    }

    #[test]
    fn wrap_chat_falls_back_when_multi_turn_fields_missing() {
        // Single-turn-only template: no assistant_turn / next_user_turn.
        // wrap_chat should still produce something usable, taking the
        // last user message and rendering it via `template`.
        let t = ChatTemplate {
            system_prompt: "sys".into(),
            template: "[S]{system}[U]{user}[A]".into(),
            assistant_turn: None,
            next_user_turn: None,
        };
        let msgs = vec![
            ChatMessage { role: ChatRole::User, content: "u1".into() },
            ChatMessage { role: ChatRole::Assistant, content: "a1".into() },
            ChatMessage { role: ChatRole::User, content: "u2".into() },
        ];
        assert_eq!(t.wrap_chat(&msgs).unwrap(), "[S]sys[U]u2[A]");
    }

    #[test]
    fn wrap_chat_returns_none_when_no_user_message() {
        let t = multi_turn_template();
        let msgs = vec![ChatMessage {
            role: ChatRole::Assistant,
            content: "a".into(),
        }];
        assert!(t.wrap_chat(&msgs).is_none());
    }

    #[test]
    fn sdk_compat_major_mismatch_errors() {
        let m = good();
        let err = m.check_sdk_compat("3.0.0").unwrap_err();
        assert!(matches!(err, ManifestError::SdkMajorMismatch { .. }));
    }

    #[test]
    fn sdk_compat_minor_drift_warns_but_passes() {
        let m = good();
        m.check_sdk_compat("2.46.1").unwrap();
    }

    #[test]
    fn sdk_compat_patch_drift_silent() {
        let m = good();
        m.check_sdk_compat("2.45.7").unwrap();
    }

    #[test]
    fn parse_semver_accepts_pre_release() {
        assert_eq!(parse_semver("2.44.0"), Some((2, 44, 0)));
        assert_eq!(parse_semver("2.44.0-rc1"), Some((2, 44, 0)));
        assert_eq!(parse_semver("2.44.0+build.5"), Some((2, 44, 0)));
        assert_eq!(parse_semver("2.44"), None);
        assert_eq!(parse_semver("two.point.four"), None);
    }
}

//! `hexrun.json` model manifest format.
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

/// Quantization schemes hexrun understands. The HTP backend on Snapdragon X
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
    /// 4-bit weights (group-quantized). Smallest footprint; quality varies
    /// by model.
    Int4,
    /// FP16 — supported on GPU/CPU EPs but generally not on HTP.
    Fp16,
}

/// A `hexrun.json` manifest.
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
    /// sha256 hex digests for each file referenced under [`Self::files`].
    /// Keyed by the field name (e.g. `"model"`, `"ctx"`, `"tokenizer"`).
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
}

/// Files referenced by a manifest. All paths are relative to the model
/// directory; absolute paths and `..` segments are rejected at validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFiles {
    /// ONNX file (path relative to the manifest's directory).
    pub model: String,
    /// Optional QNN context binary (`*.qnn_ctx.bin`) for fast cold start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx: Option<String>,
    /// HuggingFace-style `tokenizer.json` file.
    pub tokenizer: String,
    /// Optional architecture/runtime config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
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
        check_safe_relpath("files.model", &self.files.model)?;
        check_safe_relpath("files.tokenizer", &self.files.tokenizer)?;
        if let Some(ref ctx) = self.files.ctx {
            check_safe_relpath("files.ctx", ctx)?;
        }
        if let Some(ref cfg) = self.files.config {
            check_safe_relpath("files.config", cfg)?;
        }
        for (key, hex) in &self.sha256 {
            if !is_sha256_hex(hex) {
                return Err(ManifestError::Invalid(format!(
                    "sha256[{key}] is not 64 lowercase hex chars"
                )));
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
            quant: Quant::Int8WInt16A,
            qnn_sdk: "2.44.0".into(),
            files: ManifestFiles {
                model: "model.onnx".into(),
                ctx: Some("model.qnn_ctx.bin".into()),
                tokenizer: "tokenizer.json".into(),
                config: None,
            },
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
        assert_eq!(back.quant, Quant::Int8WInt16A);
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
        m.files.tokenizer = "../etc/passwd".into();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::Invalid(s) if s.contains("'..'")));
    }

    #[test]
    fn rejects_absolute_path() {
        let mut m = good();
        m.files.model = "/abs/model.onnx".into();
        assert!(matches!(m.validate(), Err(ManifestError::Invalid(_))));
    }

    #[test]
    fn rejects_drive_prefix() {
        let mut m = good();
        m.files.model = "C:\\abs\\model.onnx".into();
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
    fn sdk_compat_major_mismatch_errors() {
        let m = good();
        let err = m.check_sdk_compat("3.0.0").unwrap_err();
        assert!(matches!(err, ManifestError::SdkMajorMismatch { .. }));
    }

    #[test]
    fn sdk_compat_minor_drift_warns_but_passes() {
        let m = good();
        m.check_sdk_compat("2.45.1").unwrap();
    }

    #[test]
    fn sdk_compat_patch_drift_silent() {
        let m = good();
        m.check_sdk_compat("2.44.7").unwrap();
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

//! Inference engine: load a model bundle, run a generation loop.
//!
//! When the `genie` feature is enabled, [`Engine`] holds a `qnn::Dialog`
//! and drives the Hexagon NPU via Qualcomm's Genie LLM runtime. Without
//! the feature, `Engine::generate` returns [`EngineError::FeatureDisabled`].

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;
#[cfg(feature = "genie")]
use tracing::debug;
use tracing::info;

use crate::manifest::{Manifest, ManifestError};

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
    /// Directory containing `hexrun.json` and the files it references.
    pub model_dir: PathBuf,
    /// Backend to use.
    pub backend: Backend,
    /// Maximum tokens to generate per call. Currently advisory — Genie
    /// honours its own internal generation limits via the model config.
    pub max_tokens: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::new(),
            backend: Backend::Genie,
            max_tokens: 512,
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
    #[error("backend {0:?} is not available — rebuild hexrun-core with the appropriate feature")]
    FeatureDisabled(Backend),
    /// An error from the Genie C runtime.
    #[cfg(feature = "genie")]
    #[error(transparent)]
    Genie(#[from] qnn::GenieError),
}

/// Loaded inference engine.
pub struct Engine {
    manifest: Manifest,
    config: EngineConfig,
    #[cfg(feature = "genie")]
    dialog: qnn::Dialog,
}

impl Engine {
    /// Load a model from a directory containing `hexrun.json`.
    ///
    /// On the Genie backend, also opens the bundle's `genie_config.json`
    /// and creates the Genie dialog (the dialog stays resident in NPU
    /// shared memory until [`Engine`] is dropped).
    pub fn load(config: EngineConfig) -> Result<Arc<Self>, EngineError> {
        if !config.model_dir.is_dir() {
            return Err(EngineError::ModelDirMissing(config.model_dir.clone()));
        }
        let manifest_path = config.model_dir.join("hexrun.json");
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
    fn load_genie(manifest: Manifest, config: EngineConfig) -> Result<Arc<Self>, EngineError> {
        let genie_rel =
            manifest
                .files
                .genie_config
                .as_ref()
                .ok_or_else(|| EngineError::NotAGenieBundle {
                    path: config.model_dir.join("hexrun.json"),
                })?;
        let genie_path = config.model_dir.join(genie_rel);
        debug!(path = %genie_path.display(), "opening Genie config");
        let dialog = qnn::Dialog::from_config_file(&genie_path)?;
        Ok(Arc::new(Self {
            manifest,
            config,
            dialog,
        }))
    }

    #[cfg(not(feature = "genie"))]
    #[allow(clippy::needless_pass_by_value)]
    fn load_genie(_manifest: Manifest, _config: EngineConfig) -> Result<Arc<Self>, EngineError> {
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
}

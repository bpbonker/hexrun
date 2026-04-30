//! Inference engine: load a model, run a generation loop, stream tokens.
//!
//! Phase 2 lands the real ORT QNN EP path. This module establishes the
//! public surface so dependent crates (registry, server, cli) build now.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::mpsc;
use tracing::info;

use crate::manifest::{Manifest, ManifestError};

/// Inference backend selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    /// Default: ONNX Runtime with the QNN Execution Provider.
    #[default]
    Ort,
    /// Load a pre-built QNN context binary directly. Feature-gated.
    #[cfg(feature = "qnn-direct")]
    QnnDirect,
    /// Fall back to ORT CPU EP. Useful for the >3× tokens/sec NPU sanity
    /// check and for environments without the SDK.
    Cpu,
}

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Directory containing `hexrun.json` and the files it references.
    pub model_dir: PathBuf,
    /// Backend to use.
    pub backend: Backend,
    /// Maximum tokens to generate per call.
    pub max_tokens: usize,
    /// Sampler temperature.
    pub temperature: f32,
    /// Nucleus sampling threshold.
    pub top_p: f32,
    /// Top-k pruning (0 = disabled).
    pub top_k: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            model_dir: PathBuf::new(),
            backend: Backend::Ort,
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
        }
    }
}

/// Errors that can be raised by the engine.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Model directory does not exist.
    #[error("model directory does not exist: {0}")]
    ModelDirMissing(PathBuf),
    /// Manifest could not be loaded.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Inference path not yet implemented (Phase 2).
    #[error("inference not yet implemented for backend {0:?} (Phase 2)")]
    NotYetImplemented(Backend),
}

/// Loaded inference engine.
pub struct Engine {
    manifest: Manifest,
    config: EngineConfig,
}

impl Engine {
    /// Load a model from a directory containing `hexrun.json` plus the ONNX,
    /// context-binary, and tokenizer files referenced by the manifest.
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
            "loaded manifest"
        );
        Ok(Arc::new(Self { manifest, config }))
    }

    /// Manifest of the loaded model.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Resolved engine config.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Start a generation. Returns a stream of token strings.
    ///
    /// Phase 2 will run the ORT QNN EP forward pass + sampler loop and
    /// feed tokens into the channel.
    pub async fn generate(&self, _prompt: &str) -> Result<GenerationStream, EngineError> {
        Err(EngineError::NotYetImplemented(self.config.backend))
    }
}

/// Asynchronous stream of generated tokens.
pub struct GenerationStream {
    rx: mpsc::Receiver<String>,
}

impl GenerationStream {
    /// Receive the next token, or `None` at end of stream.
    pub async fn next(&mut self) -> Option<String> {
        self.rx.recv().await
    }
}

/// Runtime detection of the NPU. Phase 1 wires this to `qnn::Capabilities`.
pub fn npu_present(_model_dir: &Path) -> bool {
    false
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
    fn defaults_use_ort_backend() {
        assert_eq!(EngineConfig::default().backend, Backend::Ort);
    }
}

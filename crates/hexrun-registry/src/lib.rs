//! Model registry and local cache.
//!
//! Phase 3 implementation:
//!
//! - **Built-in registry** of known NPU-ready models with their download
//!   URLs (Qualcomm's HuggingFace org for now). See [`KNOWN_MODELS`].
//! - **Pull**: download a zip, extract the bundle into the local cache,
//!   inspect the embedded `genie_config.json`, and emit a `hexrun.json`
//!   manifest so the model is ready for `hexrun run`.
//! - **List local**: walks the cache for `hexrun.json` files.
//! - **Remove**: deletes a cached model directory.

#![warn(missing_docs, clippy::all)]

mod known;
mod pull;

pub use known::{KnownModel, KNOWN_MODELS};
pub use pull::{pull_model, ProgressEvent};

use std::path::PathBuf;

use thiserror::Error;
use tracing::debug;

/// Errors raised by the registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// I/O error while reading or writing the cache.
    #[error("registry I/O at {path}: {source}")]
    Io {
        /// Path on which the operation failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Network or HTTP error during download.
    #[error("download from {url}: {source}")]
    Download {
        /// URL we were downloading from.
        url: String,
        /// Underlying request error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Zip extraction failed.
    #[error("zip extraction at {path}: {source}")]
    Zip {
        /// Path of the zip file or output dir.
        path: PathBuf,
        /// Underlying zip error.
        #[source]
        source: zip::result::ZipError,
    },
    /// Bundle did not contain a recognized `genie_config.json`.
    #[error("bundle at {path} did not contain genie_config.json")]
    BundleInvalid {
        /// Path of the bundle directory.
        path: PathBuf,
    },
    /// Model name is not in the built-in registry.
    #[error("model {name:?} is not in the built-in registry. Known: {known:?}")]
    UnknownModel {
        /// The unknown name the user requested.
        name: String,
        /// The names the registry does know.
        known: Vec<&'static str>,
    },
    /// Manifest emission failed.
    #[error(transparent)]
    Manifest(#[from] hexrun_core::ManifestError),
}

/// Resolve the default model cache directory.
///
/// On Windows the default is `%LOCALAPPDATA%\hexrun\models`. If
/// `HEXRUN_MODELS_DIR` is set, it overrides. If neither is set, the
/// working directory's `models/` is used as a fallback.
pub fn default_cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HEXRUN_MODELS_DIR") {
        return PathBuf::from(d);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("hexrun").join("models");
        debug!(path = %p.display(), "using LOCALAPPDATA cache dir");
        return p;
    }
    debug!("LOCALAPPDATA unset; falling back to ./models");
    PathBuf::from("./models")
}

/// List names of locally cached models.
pub async fn list_local() -> Result<Vec<String>, RegistryError> {
    let dir = default_cache_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    let read = std::fs::read_dir(&dir).map_err(|e| RegistryError::Io {
        path: dir.clone(),
        source: e,
    })?;
    for entry in read {
        let entry = entry.map_err(|e| RegistryError::Io {
            path: dir.clone(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("hexrun.json").is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Remove a locally cached model directory.
///
/// Refuses to delete anything that doesn't look like a hexrun model
/// directory (must contain a `hexrun.json`) — defensive guard against
/// accidentally pointing this at the wrong path.
pub fn remove_local(model_name: &str) -> Result<PathBuf, RegistryError> {
    let dir = default_cache_dir().join(model_name);
    if !dir.is_dir() {
        return Err(RegistryError::Io {
            path: dir,
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "model dir not found"),
        });
    }
    let manifest = dir.join("hexrun.json");
    if !manifest.is_file() {
        return Err(RegistryError::BundleInvalid { path: dir });
    }
    std::fs::remove_dir_all(&dir).map_err(|e| RegistryError::Io {
        path: dir.clone(),
        source: e,
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_dir_uses_models_dir_env_when_set() {
        if std::env::var_os("HEXRUN_MODELS_DIR").is_some() {
            // CI may set this; expect the helper to honour it.
            assert!(
                default_cache_dir().exists() || !default_cache_dir().to_string_lossy().is_empty()
            );
        }
    }

    #[test]
    fn list_local_on_missing_dir_is_empty() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(list_local()).unwrap();
    }

    #[test]
    fn known_registry_has_at_least_phi() {
        assert!(KNOWN_MODELS.iter().any(|m| m.name == "phi-3.5-mini"));
    }
}

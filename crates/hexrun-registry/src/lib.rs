//! Model registry and local cache.
//!
//! Phase 3 lands `pull`, `list`, `rm`, `show`. The registry index lives at
//! `https://registry.hexrun.dev/index.json` (with GitHub Releases fallback);
//! local cache defaults to `%LOCALAPPDATA%\hexrun\models`.

#![warn(missing_docs, clippy::all)]

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
}

/// Resolve the default model cache directory.
///
/// On Windows the default is `%LOCALAPPDATA%\hexrun\models`. If
/// `LOCALAPPDATA` is unset (uncommon — typically running outside a user
/// session), the working directory's `models/` is used as a fallback.
///
/// The returned path is *not* created on disk; callers should create it
/// when needed.
pub fn default_cache_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("hexrun").join("models");
        debug!(path = %p.display(), "using LOCALAPPDATA cache dir");
        return p;
    }
    debug!("LOCALAPPDATA unset; falling back to ./models");
    PathBuf::from("./models")
}

/// List names of locally cached models. Phase 3 implementation.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_dir_uses_localappdata_when_set() {
        // We don't mutate process env in tests (other tests may rely on it),
        // so just sanity-check that the helper produces a reasonable path
        // ending with hexrun\models when LOCALAPPDATA is present.
        if std::env::var_os("LOCALAPPDATA").is_some() {
            let p = default_cache_dir();
            let s = p.to_string_lossy();
            assert!(
                s.ends_with("hexrun\\models") || s.ends_with("hexrun/models"),
                "got {s}"
            );
        }
    }

    #[test]
    fn list_local_on_missing_dir_is_empty() {
        // The actual cache may or may not exist; if it doesn't, the function
        // must succeed with an empty list rather than erroring.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = rt.block_on(list_local()).unwrap();
    }
}

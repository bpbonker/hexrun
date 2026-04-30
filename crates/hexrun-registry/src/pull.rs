//! `pull_model` — download a known bundle, extract it, write a hexrun.json.
//!
//! The download is streamed to disk; a progress callback is invoked for
//! each chunk so callers (the CLI) can render a progress bar. Extraction
//! is done synchronously inside `tokio::task::spawn_blocking` because the
//! `zip` crate is sync.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use hexrun_core::{ChatTemplate, Manifest, ManifestFiles, Quant};
use serde::Deserialize;
use tracing::{debug, info};

use crate::known::KnownModel;
use crate::{default_cache_dir, RegistryError};

/// Progress events emitted during `pull_model`.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// HTTP body started; total size is known if Content-Length was sent.
    Started {
        /// Total compressed bytes if the server reported them.
        total: Option<u64>,
    },
    /// Bytes have been written to the on-disk download.
    Downloaded {
        /// Total bytes downloaded so far.
        bytes: u64,
    },
    /// Extraction phase has begun.
    Extracting,
    /// hexrun.json has been written; pull is complete.
    Done {
        /// Path to the model directory containing the manifest.
        model_dir: PathBuf,
    },
}

/// Download, extract, and finalize a known model.
///
/// Resolution: the target model directory is `<cache>/<name>` where the
/// cache is [`default_cache_dir`]. The bundle is extracted under
/// `<cache>/<name>/bundle/`. A `hexrun.json` is written at
/// `<cache>/<name>/hexrun.json`.
///
/// Returns the model directory on success.
pub async fn pull_model<F>(name: &str, mut on_progress: F) -> Result<PathBuf, RegistryError>
where
    F: FnMut(ProgressEvent),
{
    let known = KnownModel::lookup(name).ok_or_else(|| RegistryError::UnknownModel {
        name: name.to_string(),
        known: super::KNOWN_MODELS.iter().map(|m| m.name).collect(),
    })?;

    let cache = default_cache_dir();
    let model_dir = cache.join(known.name);
    std::fs::create_dir_all(&model_dir).map_err(|e| RegistryError::Io {
        path: model_dir.clone(),
        source: e,
    })?;

    let zip_path = model_dir.join(".pull.zip");
    download(known.url, &zip_path, &mut on_progress).await?;

    on_progress(ProgressEvent::Extracting);
    let bundle_root = extract_zip(&zip_path, &model_dir.join("bundle"))?;
    let _ = std::fs::remove_file(&zip_path);

    write_manifest(known, &model_dir, &bundle_root)?;

    on_progress(ProgressEvent::Done {
        model_dir: model_dir.clone(),
    });
    info!(name = %known.name, dir = %model_dir.display(), "pull complete");
    Ok(model_dir)
}

async fn download<F>(url: &str, dest: &Path, on_progress: &mut F) -> Result<(), RegistryError>
where
    F: FnMut(ProgressEvent),
{
    debug!(url = %url, dest = %dest.display(), "starting download");
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| RegistryError::Download {
            url: url.to_string(),
            source: Box::new(e),
        })?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| RegistryError::Download {
            url: url.to_string(),
            source: Box::new(e),
        })?;
    let resp = resp
        .error_for_status()
        .map_err(|e| RegistryError::Download {
            url: url.to_string(),
            source: Box::new(e),
        })?;

    let total = resp.content_length();
    on_progress(ProgressEvent::Started { total });

    let mut file = BufWriter::new(File::create(dest).map_err(|e| RegistryError::Io {
        path: dest.to_path_buf(),
        source: e,
    })?);

    let mut stream = resp.bytes_stream();
    let mut bytes_seen: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| RegistryError::Download {
            url: url.to_string(),
            source: Box::new(e),
        })?;
        file.write_all(&chunk).map_err(|e| RegistryError::Io {
            path: dest.to_path_buf(),
            source: e,
        })?;
        bytes_seen += chunk.len() as u64;
        on_progress(ProgressEvent::Downloaded { bytes: bytes_seen });
    }
    file.flush().map_err(|e| RegistryError::Io {
        path: dest.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Extract a zip into `dest`, then locate the directory that contains
/// `genie_config.json` (the published bundles wrap their content in a
/// single subdirectory like
/// `phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite/`).
/// Returns that subdirectory.
fn extract_zip(zip_path: &Path, dest: &Path) -> Result<PathBuf, RegistryError> {
    std::fs::create_dir_all(dest).map_err(|e| RegistryError::Io {
        path: dest.to_path_buf(),
        source: e,
    })?;

    let file = File::open(zip_path).map_err(|e| RegistryError::Io {
        path: zip_path.to_path_buf(),
        source: e,
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| RegistryError::Zip {
        path: zip_path.to_path_buf(),
        source: e,
    })?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| RegistryError::Zip {
            path: zip_path.to_path_buf(),
            source: e,
        })?;
        let entry_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };
        // Skip macOS resource fork debris that often ships in zips.
        if entry_path
            .components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new("__MACOSX"))
        {
            continue;
        }
        let out_path = dest.join(&entry_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| RegistryError::Io {
                path: out_path.clone(),
                source: e,
            })?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| RegistryError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            let mut out = File::create(&out_path).map_err(|e| RegistryError::Io {
                path: out_path.clone(),
                source: e,
            })?;
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf).map_err(|e| RegistryError::Io {
                path: out_path.clone(),
                source: e,
            })?;
            out.write_all(&buf).map_err(|e| RegistryError::Io {
                path: out_path.clone(),
                source: e,
            })?;
        }
    }

    find_bundle_root(dest)
}

/// Walk `dest` looking for a directory that contains `genie_config.json`.
fn find_bundle_root(dest: &Path) -> Result<PathBuf, RegistryError> {
    if dest.join("genie_config.json").is_file() {
        return Ok(dest.to_path_buf());
    }
    for entry in std::fs::read_dir(dest).map_err(|e| RegistryError::Io {
        path: dest.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| RegistryError::Io {
            path: dest.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("genie_config.json").is_file() {
            return Ok(path);
        }
    }
    Err(RegistryError::BundleInvalid {
        path: dest.to_path_buf(),
    })
}

/// Auto-generate the hexrun.json from the extracted bundle.
fn write_manifest(
    known: &KnownModel,
    model_dir: &Path,
    bundle_root: &Path,
) -> Result<(), RegistryError> {
    let genie_config_path = bundle_root.join("genie_config.json");
    let genie_json =
        std::fs::read_to_string(&genie_config_path).map_err(|e| RegistryError::Io {
            path: genie_config_path.clone(),
            source: e,
        })?;
    let parsed: GenieConfigPeek = serde_json::from_str(&genie_json).map_err(|e| {
        RegistryError::BundleInvalid {
            path: genie_config_path.clone(),
        }
        .pipe(|err| {
            tracing::error!(error = %e, "failed to parse genie_config.json for manifest peek");
            err
        })
    })?;

    let context = parsed.dialog.context.size;
    let vocab = parsed.dialog.context.n_vocab;

    let bundle_rel = bundle_root
        .strip_prefix(model_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("bundle"));

    let manifest = Manifest {
        name: known.name.to_string(),
        version: "0.1.0".to_string(),
        arch: known.arch.to_string(),
        vocab,
        context,
        quant: parse_quant(known.quant),
        qnn_sdk: known.qairt_version.to_string(),
        files: ManifestFiles {
            model: None,
            ctx: None,
            tokenizer: Some(rel_join(&bundle_rel, "tokenizer.json")),
            config: None,
            genie_config: Some(rel_join(&bundle_rel, "genie_config.json")),
        },
        chat_template: Some(ChatTemplate {
            system_prompt: known.chat_template.system_prompt.to_string(),
            template: known.chat_template.template.to_string(),
        }),
        sha256: BTreeMap::new(),
    };

    let manifest_path = model_dir.join("hexrun.json");
    let serialized = serde_json::to_string_pretty(&manifest).map_err(|e| RegistryError::Io {
        path: manifest_path.clone(),
        source: std::io::Error::other(e),
    })?;
    std::fs::write(&manifest_path, serialized).map_err(|e| RegistryError::Io {
        path: manifest_path.clone(),
        source: e,
    })?;
    info!(path = %manifest_path.display(), "wrote manifest");
    Ok(())
}

fn rel_join(parent: &Path, child: &str) -> String {
    parent
        .join(child)
        .to_string_lossy()
        .replace('\\', "/")
        .to_string()
}

fn parse_quant(s: &str) -> Quant {
    match s {
        "w4a16" => Quant::W4A16,
        "w8a16" => Quant::W8A16,
        "int8-w-int16-a" => Quant::Int8WInt16A,
        "int8" => Quant::Int8,
        "int4" => Quant::Int4,
        "fp16" => Quant::Fp16,
        // Default conservatively if we don't recognize it.
        _ => Quant::W8A16,
    }
}

#[derive(Deserialize)]
struct GenieConfigPeek {
    dialog: GenieDialogPeek,
}

#[derive(Deserialize)]
struct GenieDialogPeek {
    context: GenieContextPeek,
}

#[derive(Deserialize)]
struct GenieContextPeek {
    size: u32,
    #[serde(rename = "n-vocab")]
    n_vocab: u32,
}

trait Pipe: Sized {
    fn pipe<F, T>(self, f: F) -> T
    where
        F: FnOnce(Self) -> T,
    {
        f(self)
    }
}
impl<T> Pipe for T {}

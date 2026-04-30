use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use hexrun_core::{Engine, EngineConfig};

#[derive(Parser)]
#[command(
    name = "hexrun",
    about = "NPU-first local LLM runtime for Snapdragon X Elite",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download a model from the registry (Phase 3 — not yet implemented)
    Pull { model: String },
    /// List locally cached models
    List,
    /// Remove a locally cached model (Phase 3 — not yet implemented)
    Rm { model: String },
    /// Show the manifest of a locally cached model
    Show {
        /// Model name (e.g. "phi-3.5-mini") or absolute path to a model directory
        model: String,
        /// Print extra runtime/profile info if available
        #[arg(long)]
        profile: bool,
    },
    /// Run a one-shot generation against a locally cached model
    Run {
        /// Model name (e.g. "phi-3.5-mini") or absolute path to a model directory
        model: String,
        /// Prompt text. If multiple words, quote them or pass as trailing args.
        #[arg(trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// List in-flight sessions on a running `hexrun serve` (Phase 4)
    Ps,
    /// Start the OpenAI- and Ollama-compatible HTTP server. The named model
    /// is loaded on startup and stays resident in NPU shared memory for
    /// the lifetime of the server.
    Serve {
        /// Model name (resolved like `hexrun run`) or absolute path
        #[arg(long)]
        model: Option<String>,
        /// Bind address. Default 127.0.0.1:11435 so hexrun and Ollama can
        /// run side-by-side (Ollama defaults to 11434).
        #[arg(long, default_value = "127.0.0.1:11435")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hexrun=info")),
        )
        .init();

    // Make Genie.dll, QnnHtp.dll and the Hexagon stubs reachable at process
    // startup. Done unconditionally because cargo-run inherits this
    // process's environment; if the user already set these we don't need
    // to overwrite.
    if let Ok(qairt) = env::var("QNN_SDK_ROOT") {
        prepend_qairt_paths(&qairt);
    }

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Pull { model } => pull_model(&model).await?,
        Cmd::List => list_models()?,
        Cmd::Rm { model } => remove_model(&model)?,
        Cmd::Show { model, profile } => show_model(&model, profile)?,
        Cmd::Run { model, prompt } => {
            let prompt = prompt.join(" ");
            run_model(&model, &prompt)?
        }
        Cmd::Ps => {
            println!("ps: (Phase 4 — not yet implemented)");
        }
        Cmd::Serve { model, bind } => {
            let state = match model.as_deref() {
                Some(name) => {
                    let dir = resolve_model_dir(name)?;
                    let cfg = EngineConfig {
                        model_dir: dir,
                        ..Default::default()
                    };
                    let load_started = Instant::now();
                    let engine = Engine::load(cfg)?;
                    eprintln!(
                        "[loaded {} ({}, {:?}, ctx {}) in {:.2?}]",
                        engine.manifest().name,
                        engine.manifest().arch,
                        engine.manifest().quant,
                        engine.manifest().context,
                        load_started.elapsed()
                    );
                    let model_name = engine.manifest().name.clone();
                    let arc_engine = std::sync::Arc::new(std::sync::Mutex::new(engine));
                    hexrun_server::ServerState {
                        engine: Some(arc_engine),
                        model_name: Some(model_name),
                    }
                }
                None => {
                    eprintln!(
                        "warning: starting hexrun serve without --model. Endpoints will return 503 \
                         until a model is loaded."
                    );
                    hexrun_server::ServerState::default()
                }
            };
            eprintln!(
                "hexrun serve listening on http://{}\n  - OpenAI-compatible: POST /v1/chat/completions, GET /v1/models\n  - Ollama-compatible: POST /api/generate, POST /api/chat, GET /api/tags\n  - GET /healthz",
                bind
            );
            hexrun_server::serve(bind, state).await?;
        }
    }
    Ok(())
}

/// Resolve a user-supplied model identifier to a model directory.
///
/// Resolution order:
///   1. If the identifier is an absolute path that's an existing directory, use it as-is.
///   2. If `HEXRUN_MODELS_DIR` is set, look for `$HEXRUN_MODELS_DIR/<name>`.
///   3. Default cache dir: `%LOCALAPPDATA%\hexrun\models\<name>`.
fn resolve_model_dir(model: &str) -> Result<PathBuf> {
    let p = PathBuf::from(model);
    if p.is_absolute() && p.is_dir() {
        return Ok(p);
    }
    if let Some(base) = env::var_os("HEXRUN_MODELS_DIR") {
        let candidate = PathBuf::from(base).join(model);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    if let Some(base) = env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(base)
            .join("hexrun")
            .join("models")
            .join(model);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "model {:?} not found. Set HEXRUN_MODELS_DIR to a directory containing a {model:?} \
         subfolder with a hexrun.json, or use an absolute path.",
        model
    ))
}

fn list_models() -> Result<()> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = env::var_os("HEXRUN_MODELS_DIR") {
        roots.push(PathBuf::from(d));
    }
    if let Some(d) = env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(d).join("hexrun").join("models"));
    }
    if roots.is_empty() {
        println!("no model search paths set (HEXRUN_MODELS_DIR or LOCALAPPDATA)");
        return Ok(());
    }
    let mut found = 0;
    for root in &roots {
        if !root.is_dir() {
            continue;
        }
        for entry in
            std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("hexrun.json");
            if !manifest_path.is_file() {
                continue;
            }
            match hexrun_core::Manifest::read(&manifest_path) {
                Ok(m) => {
                    println!(
                        "{name:30} {arch:8} {quant:14} {context:>5} ctx   {dir}",
                        name = m.name,
                        arch = m.arch,
                        quant = format!("{:?}", m.quant),
                        context = m.context,
                        dir = path.display(),
                    );
                    found += 1;
                }
                Err(e) => {
                    eprintln!("{}: invalid manifest: {e}", manifest_path.display());
                }
            }
        }
    }
    if found == 0 {
        println!("no models found in:");
        for r in &roots {
            println!("  {}", r.display());
        }
    }
    Ok(())
}

fn show_model(model: &str, profile: bool) -> Result<()> {
    let dir = resolve_model_dir(model)?;
    let manifest_path = dir.join("hexrun.json");
    let m = hexrun_core::Manifest::read(&manifest_path)?;
    println!("{:>14}  {}", "name:", m.name);
    println!("{:>14}  {}", "version:", m.version);
    println!("{:>14}  {}", "arch:", m.arch);
    println!("{:>14}  {:?}", "quant:", m.quant);
    println!("{:>14}  {}", "vocab:", m.vocab);
    println!("{:>14}  {}", "context:", m.context);
    println!("{:>14}  {}", "qnn_sdk:", m.qnn_sdk);
    println!("{:>14}  {}", "directory:", dir.display());
    if let Some(ref gc) = m.files.genie_config {
        println!("{:>14}  {}", "genie_config:", gc);
    }
    if let Some(ref ct) = m.chat_template {
        println!("{:>14}  {:?}", "chat_template:", ct.template);
    }
    if profile {
        println!();
        println!("(profile flag is reserved for runtime stats — Phase 4 will populate it)");
    }
    Ok(())
}

async fn pull_model(name: &str) -> Result<()> {
    use hexrun_registry::{pull_model as registry_pull, KnownModel, ProgressEvent, KNOWN_MODELS};

    if KnownModel::lookup(name).is_none() {
        eprintln!("model {name:?} is not in the built-in registry. Known models:");
        for m in KNOWN_MODELS {
            eprintln!(
                "  {} ({} {}, ~{:.1} GB)",
                m.name,
                m.arch,
                m.quant,
                m.size_estimate_bytes as f64 / 1e9
            );
        }
        return Err(anyhow!("unknown model"));
    }

    let pb = indicatif::ProgressBar::new(0);
    pb.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.cyan} {msg:30} [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} eta {eta}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );
    pb.set_message(format!("pull {name}"));
    let pb_clone = pb.clone();

    let model_dir = registry_pull(name, move |evt| match evt {
        ProgressEvent::Started { total } => {
            if let Some(t) = total {
                pb_clone.set_length(t);
            } else {
                pb_clone.set_length(0);
            }
        }
        ProgressEvent::Downloaded { bytes } => {
            pb_clone.set_position(bytes);
        }
        ProgressEvent::Extracting => {
            pb_clone.set_message("extracting".to_string());
        }
        ProgressEvent::Done { .. } => {
            pb_clone.finish_with_message("done");
        }
    })
    .await?;

    println!("\nmodel ready at {}", model_dir.display());
    println!("verify it: hexrun show {name}\nrun it:    hexrun run {name} \"hello\"");
    Ok(())
}

fn remove_model(name: &str) -> Result<()> {
    let removed = hexrun_registry::remove_local(name)?;
    println!("removed {}", removed.display());
    Ok(())
}

fn run_model(model: &str, prompt: &str) -> Result<()> {
    if prompt.is_empty() {
        return Err(anyhow!("prompt is empty; provide one as trailing argument"));
    }
    let dir = resolve_model_dir(model)?;
    let cfg = EngineConfig {
        model_dir: dir.clone(),
        ..Default::default()
    };
    let load_started = Instant::now();
    let engine = Engine::load(cfg)?;
    let load_elapsed = load_started.elapsed();
    eprintln!(
        "[loaded {} ({}, {:?}, ctx {}) in {load_elapsed:.2?}]",
        engine.manifest().name,
        engine.manifest().arch,
        engine.manifest().quant,
        engine.manifest().context,
    );

    let infer_started = Instant::now();
    let mut chunks_seen = 0usize;
    engine.generate_streaming(prompt, |chunk| {
        chunks_seen += 1;
        print!("{chunk}");
        std::io::stdout().flush().ok();
    })?;
    let infer_elapsed = infer_started.elapsed();
    println!();
    eprintln!(
        "[generated {chunks_seen} chunks in {infer_elapsed:.2?}; ~{:.1} chunks/s]",
        chunks_seen as f64 / infer_elapsed.as_secs_f64()
    );
    Ok(())
}

/// Prepend QAIRT bin/lib to PATH and set ADSP_LIBRARY_PATH so DLLs and
/// Hexagon stubs resolve at process startup. Mirrors `scripts\dev-shell.bat`.
fn prepend_qairt_paths(qairt: &str) {
    let qairt_path = Path::new(qairt);
    let bin = qairt_path.join("bin").join("aarch64-windows-msvc");
    let lib = qairt_path.join("lib").join("aarch64-windows-msvc");
    let adsp = qairt_path.join("lib").join("hexagon-v73").join("unsigned");

    if !bin.is_dir() || !lib.is_dir() || !adsp.is_dir() {
        // QAIRT layout looks unfamiliar; bail without overwriting env.
        return;
    }

    let path_var = env::var_os("PATH").unwrap_or_default();
    let mut new_path = OsString::new();
    new_path.push(&bin);
    new_path.push(";");
    new_path.push(&lib);
    new_path.push(";");
    new_path.push(&path_var);
    env::set_var("PATH", new_path);
    if env::var_os("ADSP_LIBRARY_PATH").is_none() {
        env::set_var("ADSP_LIBRARY_PATH", &adsp);
    }
}

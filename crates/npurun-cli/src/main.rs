use std::env;
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use npurun_core::{Engine, EngineConfig};

#[derive(Parser)]
#[command(
    name = "npurun",
    about = "NPU-first local LLM runtime for Snapdragon X Elite",
    version
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download a model from the built-in registry. Sha256-verified, resumable.
    Pull { model: String },
    /// List locally cached models
    List,
    /// Remove a locally cached model from disk
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
    /// Print versions of npurun, libGenie, and the QAIRT SDK in a single shot
    Version,
    /// Probe a running `npurun serve` and report what model it has loaded.
    /// Connects to GET /healthz on `--addr` (default 127.0.0.1:11435).
    Ps {
        /// host:port of the running npurun serve.
        #[arg(long, default_value = "127.0.0.1:11435")]
        addr: String,
        /// Bearer token, if the target server was started with --auth-token.
        #[arg(long, value_name = "TOKEN")]
        auth_token: Option<String>,
    },
    /// Run a fixed-prompt warm-query benchmark against a locally cached model
    Bench {
        /// Model name (e.g. "phi-3.5-mini") or absolute path to a model directory
        model: String,
        /// Override the default prompt set with a single custom prompt.
        /// If omitted, four built-in prompts are used.
        #[arg(long)]
        prompt: Option<String>,
        /// Number of times to run each prompt (defaults to 1).
        #[arg(long, default_value_t = 1)]
        repeats: usize,
        /// Skip the first query when computing the warm-summary aggregate.
        /// Defaults to true; pass --no-skip-first to include it.
        #[arg(long, default_value_t = true)]
        skip_first: bool,
    },
    /// Start the OpenAI- and Ollama-compatible HTTP server. The named model
    /// is loaded on startup and stays resident in NPU shared memory for
    /// the lifetime of the server.
    Serve {
        /// Model name (resolved like `npurun run`) or absolute path
        #[arg(long)]
        model: Option<String>,
        /// Bind address. Default 127.0.0.1:11435 so npurun and Ollama can
        /// run side-by-side (Ollama defaults to 11434). Use `0.0.0.0:11435`
        /// to expose on the LAN; you'll be warned and you should pair
        /// that with --auth-token.
        #[arg(long, default_value = "127.0.0.1:11435")]
        bind: SocketAddr,
        /// Require `Authorization: Bearer <token>` on /v1/* and /api/*.
        /// Strongly recommended whenever --bind is non-loopback.
        #[arg(long, value_name = "TOKEN")]
        auth_token: Option<String>,
        /// Skip the post-load warmup query. Without this, the server
        /// runs a tiny generation before accepting requests so the first
        /// real client doesn't pay the cold-start cost on top of the
        /// 9–30 second bundle load.
        #[arg(long)]
        no_warmup: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,npurun=info")),
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
        Cmd::Version => print_versions()?,
        Cmd::Bench {
            model,
            prompt,
            repeats,
            skip_first,
        } => bench_model(&model, prompt.as_deref(), repeats, skip_first)?,
        Cmd::Ps { addr, auth_token } => probe_ps(&addr, auth_token.as_deref()).await?,
        Cmd::Serve {
            model,
            bind,
            auth_token,
            no_warmup,
        } => {
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
                    if !no_warmup {
                        let warmup_started = Instant::now();
                        match engine.generate("Hi.") {
                            Ok(_) => {
                                eprintln!("[warmup query in {:.2?}]", warmup_started.elapsed());
                            }
                            Err(e) => {
                                eprintln!("[warmup failed: {e}; continuing]");
                            }
                        }
                    }
                    let model_name = engine.manifest().name.clone();
                    let arc_engine = std::sync::Arc::new(engine);
                    npurun_server::ServerState {
                        engine: Some(arc_engine),
                        inference_permit: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
                        model_name: Some(model_name),
                        started_at: Some(std::time::SystemTime::now()),
                        auth_token: auth_token.clone(),
                    }
                }
                None => {
                    eprintln!(
                        "warning: starting npurun serve without --model. Endpoints will return 503 \
                         until a model is loaded."
                    );
                    npurun_server::ServerState {
                        inference_permit: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
                        started_at: Some(std::time::SystemTime::now()),
                        auth_token: auth_token.clone(),
                        ..Default::default()
                    }
                }
            };

            if !bind.ip().is_loopback() {
                eprintln!(
                    "\n  ⚠  exposing npurun on {bind} — anyone who can reach this address can use this server.\n     {}\n",
                    if auth_token.is_some() {
                        "auth token enabled (clients must send `Authorization: Bearer <token>`)"
                    } else {
                        "no --auth-token set; consider passing one when binding to a non-loopback address"
                    }
                );
            }
            eprintln!(
                "npurun serve listening on http://{}\n  - OpenAI-compatible: POST /v1/chat/completions, GET /v1/models\n  - Ollama-compatible: POST /api/generate, POST /api/chat, GET /api/tags\n  - GET /healthz",
                bind
            );
            npurun_server::serve(bind, state).await?;
        }
    }
    Ok(())
}

/// Resolve a user-supplied model identifier to a model directory.
///
/// Resolution order:
///   1. If the identifier is an absolute path that's an existing directory, use it as-is.
///   2. If `NPURUN_MODELS_DIR` is set, look for `$NPURUN_MODELS_DIR/<name>`.
///   3. Default cache dir: `%LOCALAPPDATA%\npurun\models\<name>`.
///
/// Accepts Ollama-style `<name>:<tag>` references (e.g.
/// `phi-3.5-mini:latest`). The tag is stripped before resolution —
/// npurun does not version cached bundles.
fn resolve_model_dir(model: &str) -> Result<PathBuf> {
    let bare = model.split(':').next().unwrap_or(model);
    let p = PathBuf::from(bare);
    if p.is_absolute() && p.is_dir() {
        return Ok(p);
    }
    if let Some(base) = env::var_os("NPURUN_MODELS_DIR") {
        let candidate = PathBuf::from(base).join(bare);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    if let Some(base) = env::var_os("LOCALAPPDATA") {
        let candidate = PathBuf::from(base).join("npurun").join("models").join(bare);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "model {bare:?} not found. Set NPURUN_MODELS_DIR to a directory containing a {bare:?} \
         subfolder with a npurun.json, or use an absolute path.",
    ))
}

fn list_models() -> Result<()> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = env::var_os("NPURUN_MODELS_DIR") {
        roots.push(PathBuf::from(d));
    }
    if let Some(d) = env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(d).join("npurun").join("models"));
    }
    if roots.is_empty() {
        println!("no model search paths set (NPURUN_MODELS_DIR or LOCALAPPDATA)");
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
            let manifest_path = path.join("npurun.json");
            if !manifest_path.is_file() {
                continue;
            }
            match npurun_core::Manifest::read(&manifest_path) {
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
    let manifest_path = dir.join("npurun.json");
    let m = npurun_core::Manifest::read(&manifest_path)?;
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
    use npurun_registry::{pull_model as registry_pull, KnownModel, ProgressEvent, KNOWN_MODELS};

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

    // TTY-aware progress: indicatif progress bar in interactive terminals,
    // periodic log lines in non-interactive ones (CI, script pipes).
    let interactive = std::io::stderr().is_terminal();
    let model_dir = if interactive {
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
        registry_pull(name, move |evt| match evt {
            ProgressEvent::Started { total } => pb_clone.set_length(total.unwrap_or(0)),
            ProgressEvent::Resuming { already_have } => {
                pb_clone.set_message(format!("resuming from {:.2} GB", already_have as f64 / 1e9));
            }
            ProgressEvent::Downloaded { bytes } => pb_clone.set_position(bytes),
            ProgressEvent::Extracting => pb_clone.set_message("extracting".to_string()),
            ProgressEvent::Done { sha256, .. } => {
                pb_clone.finish_with_message(format!("done sha256={}…", &sha256[..12]))
            }
        })
        .await?
    } else {
        let started = Instant::now();
        let mut total: Option<u64> = None;
        let mut last_logged = started;
        registry_pull(name, move |evt| match evt {
            ProgressEvent::Started { total: t } => {
                total = t;
                eprintln!(
                    "[pull] starting download (size: {})",
                    t.map(|s| format!("{:.2} GB", s as f64 / 1e9))
                        .unwrap_or_else(|| "unknown".to_string())
                );
            }
            ProgressEvent::Resuming { already_have } => {
                eprintln!(
                    "[pull] resuming from {:.2} GB already on disk",
                    already_have as f64 / 1e9
                );
            }
            ProgressEvent::Downloaded { bytes } => {
                if last_logged.elapsed() > Duration::from_secs(5) {
                    let pct = total
                        .map(|t| (bytes as f64 / t as f64) * 100.0)
                        .unwrap_or(0.0);
                    let secs = started.elapsed().as_secs_f64();
                    let mbps = (bytes as f64 / 1e6) / secs.max(0.001);
                    eprintln!(
                        "[pull] {pct:.1}% ({:.2} GB) at {mbps:.1} MB/s",
                        bytes as f64 / 1e9
                    );
                    last_logged = Instant::now();
                }
            }
            ProgressEvent::Extracting => eprintln!("[pull] extracting..."),
            ProgressEvent::Done { sha256, .. } => {
                eprintln!("[pull] done in {:.2?}", started.elapsed());
                eprintln!("[pull] sha256={sha256}");
            }
        })
        .await?
    };

    println!("\nmodel ready at {}", model_dir.display());
    println!("verify it: npurun show {name}\nrun it:    npurun run {name} \"hello\"");
    Ok(())
}

fn remove_model(name: &str) -> Result<()> {
    let removed = npurun_registry::remove_local(name)?;
    println!("removed {}", removed.display());
    Ok(())
}

async fn probe_ps(addr: &str, auth_token: Option<&str>) -> Result<()> {
    // /healthz is intentionally unauthenticated — clients can probe
    // server identity without holding a token. Auth-token support is
    // here for future endpoints we may surface (e.g. /api/ps once the
    // server gains a sessions list).
    let url = format!("http://{addr}/healthz");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .context("building HTTP client")?;
    let mut req = client.get(&url);
    if let Some(tok) = auth_token {
        req = req.bearer_auth(tok);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            println!("no npurun serve responding at {addr} ({e})");
            return Ok(());
        }
    };
    let status = resp.status();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            println!("npurun serve at {addr} responded {status} but the body could not be read ({e})");
            return Ok(());
        }
    };
    let body: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            println!("npurun serve at {addr} responded {status} but the body was not JSON ({e})");
            return Ok(());
        }
    };
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("(none loaded)");
    let uptime = body
        .get("uptime_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let auth_on = body
        .get("auth")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let version = body
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let server_status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    println!("npurun serve at http://{addr}");
    println!("  status:    {server_status}");
    println!("  model:     {model}");
    println!("  uptime:    {}", format_uptime(uptime));
    println!("  auth:      {}", if auth_on { "bearer-token" } else { "none" });
    println!("  version:   {version}");
    Ok(())
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn print_versions() -> Result<()> {
    println!("npurun       {}", env!("CARGO_PKG_VERSION"));
    let v = qnn::api_version();
    println!("libGenie     {}.{}.{}", v.major, v.minor, v.patch);
    if let Ok(root) = env::var("QNN_SDK_ROOT") {
        let sdk_yaml = Path::new(&root).join("sdk.yaml");
        let mut shown = false;
        if let Ok(text) = std::fs::read_to_string(&sdk_yaml) {
            for line in text.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("version:") {
                    println!("QAIRT SDK    {}  ({})", rest.trim(), root);
                    shown = true;
                    break;
                }
            }
        }
        if !shown {
            println!("QAIRT SDK    (sdk.yaml not parseable; root: {root})");
        }
    } else {
        println!("QAIRT SDK    (QNN_SDK_ROOT not set)");
    }
    Ok(())
}

const BENCH_PROMPTS: &[&str] = &[
    "Write a one-line joke about Snapdragon laptops.",
    "Briefly explain why an NPU is more energy-efficient than a CPU for matrix multiplication.",
    "List three reasons running language models locally on a laptop is useful.",
    "What is 17 multiplied by 23? Just the number.",
];

fn approx_token_count(s: &str) -> usize {
    let words = s.split_whitespace().count();
    ((words as f64) * 1.3).round() as usize
}

fn bench_model(
    model: &str,
    custom_prompt: Option<&str>,
    repeats: usize,
    skip_first: bool,
) -> Result<()> {
    let dir = resolve_model_dir(model)?;
    let cfg = EngineConfig {
        model_dir: dir,
        ..Default::default()
    };
    let load_started = Instant::now();
    let engine = Engine::load(cfg)?;
    let load_elapsed = load_started.elapsed();
    eprintln!(
        "==  npurun bench: {} ({}, {:?}, ctx {})  ==",
        engine.manifest().name,
        engine.manifest().arch,
        engine.manifest().quant,
        engine.manifest().context,
    );
    eprintln!("[bundle loaded in {load_elapsed:.2?}]");

    let prompts: Vec<&str> = match custom_prompt {
        Some(p) => vec![p],
        None => BENCH_PROMPTS.to_vec(),
    };
    let total_runs = prompts.len() * repeats.max(1);
    eprintln!("[running {total_runs} queries]\n");

    let mut runs: Vec<RunStat> = Vec::with_capacity(total_runs);
    let mut idx = 0usize;
    for _ in 0..repeats.max(1) {
        for prompt in &prompts {
            idx += 1;
            let started = Instant::now();
            let mut output = String::new();
            let mut first_token_at: Option<Duration> = None;
            engine.generate_streaming(prompt, |chunk| {
                if first_token_at.is_none() && !chunk.is_empty() {
                    first_token_at = Some(started.elapsed());
                }
                output.push_str(chunk);
            })?;
            let total = started.elapsed();
            let ttft = first_token_at.unwrap_or(total);
            let tokens = approx_token_count(&output);
            let gen_time = total.saturating_sub(ttft);
            let tps_post = if gen_time.as_secs_f64() > 0.0 {
                tokens as f64 / gen_time.as_secs_f64()
            } else {
                0.0
            };
            println!("--- query {idx} ---");
            println!("    prompt: {prompt}");
            println!("    response ({tokens} approx tokens): {}", output.trim());
            println!("    total: {total:.2?}   ttft: {ttft:.2?}   gen: {gen_time:.2?}   tok/s post-ttft: {tps_post:.1}");
            println!();
            runs.push(RunStat {
                total,
                ttft,
                gen_time,
                tokens,
            });
        }
    }

    let warm: Vec<&RunStat> = if skip_first && runs.len() > 1 {
        runs.iter().skip(1).collect()
    } else {
        runs.iter().collect()
    };
    if warm.is_empty() {
        return Ok(());
    }
    let n = warm.len() as u32;
    let avg = |f: fn(&RunStat) -> Duration| -> Duration {
        warm.iter().map(|r| f(r)).sum::<Duration>() / n
    };
    let total_tokens: usize = warm.iter().map(|r| r.tokens).sum();
    let total_secs: f64 = warm.iter().map(|r| r.total.as_secs_f64()).sum();
    let total_gen_secs: f64 = warm.iter().map(|r| r.gen_time.as_secs_f64()).sum();
    let label = if skip_first {
        "warm summary (skipping first query)"
    } else {
        "summary"
    };
    println!("==  {label}  ==");
    println!("    queries:                  {}", warm.len());
    println!("    avg total per query:      {:.2?}", avg(|r| r.total));
    println!("    avg time-to-first-token:  {:.2?}", avg(|r| r.ttft));
    println!("    avg generation time:      {:.2?}", avg(|r| r.gen_time));
    println!(
        "    aggregate tok/s (incl ttft): {:.1}",
        total_tokens as f64 / total_secs
    );
    println!(
        "    aggregate tok/s (post ttft): {:.1}",
        total_tokens as f64 / total_gen_secs
    );
    Ok(())
}

#[derive(Debug)]
struct RunStat {
    total: Duration,
    ttft: Duration,
    gen_time: Duration,
    tokens: usize,
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

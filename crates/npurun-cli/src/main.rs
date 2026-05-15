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
    about = "NPU-first local LLM runtime for Snapdragon X-series Windows-on-ARM laptops",
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
    /// Run a one-shot generation. By default cold-loads the bundle into
    /// this process; pass `--addr <host:port>` to dispatch the prompt to
    /// a running `npurun serve` instead, skipping the 9–11 s bundle load.
    Run {
        /// Model name (e.g. "phi-3.5-mini") or absolute path to a model directory
        model: String,
        /// Prompt text. If multiple words, quote them or pass as trailing args.
        #[arg(trailing_var_arg = true)]
        prompt: Vec<String>,
        /// host:port of a running `npurun serve` to dispatch the prompt
        /// to. When set, `run` POSTs to `/v1/chat/completions` and
        /// streams the reply, skipping the local bundle load.
        /// Falls back to the env var `NPURUN_SERVE_ADDR` if unset.
        #[arg(long, value_name = "ADDR", env = "NPURUN_SERVE_ADDR")]
        addr: Option<String>,
        /// Bearer token, if `--addr` points at a server started with
        /// `--auth-token`.
        #[arg(long, value_name = "TOKEN")]
        auth_token: Option<String>,
    },
    /// Print versions of npurun, libGenie, and the QAIRT SDK in a single shot
    Version,
    /// Probe and report the local NPU hardware: SoC name, Hexagon architecture,
    /// Qualcomm AI Engine PnP device, QAIRT SDK + libGenie versions. Does not
    /// gate on SoC strings — npurun runs anywhere libGenie loads, including
    /// X Plus and X 10-core variants that other NPU runtimes refuse.
    ShowHardware,
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
        /// Pin the Genie context tier for this run (one of the bundle's
        /// compiled `clNNNN` values, e.g. 512, 1024, 2048, 4096 for
        /// Phi 3.5 Mini). Errors with the available tier list if the
        /// requested value isn't compiled in. When omitted, the bundle's
        /// manifest-declared context is used.
        #[arg(long, value_name = "N")]
        ctx: Option<u32>,
        /// Append one row per (prompt, repeat) to a CSV at this path.
        /// Header columns:
        /// `model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft`.
        /// The header is written once when the file is created;
        /// subsequent runs append.
        #[arg(long, value_name = "PATH")]
        csv: Option<PathBuf>,
        /// Stress mode: keep cycling through the prompt set until at
        /// least N seconds of wall-clock have elapsed. Overrides
        /// `--repeats`. Reports extended stats — min/median/max tok/s,
        /// std-dev, and a first-half-vs-second-half degradation percent
        /// to catch thermal throttling. Use 300+ for a meaningful
        /// thermal window on Snapdragon X.
        #[arg(long, value_name = "SECS")]
        duration: Option<u64>,
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
        Cmd::Run {
            model,
            prompt,
            addr,
            auth_token,
        } => {
            let prompt = prompt.join(" ");
            match addr {
                Some(addr) => run_remote(&addr, &model, &prompt, auth_token.as_deref()).await?,
                None => run_model(&model, &prompt)?,
            }
        }
        Cmd::Version => print_versions()?,
        Cmd::ShowHardware => show_hardware()?,
        Cmd::Bench {
            model,
            prompt,
            repeats,
            skip_first,
            ctx,
            csv,
            duration,
        } => bench_model(
            &model,
            prompt.as_deref(),
            repeats,
            skip_first,
            ctx,
            csv.as_deref(),
            duration,
        )?,
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
            println!(
                "npurun serve at {addr} responded {status} but the body could not be read ({e})"
            );
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
    let auth_on = body.get("auth").and_then(|v| v.as_bool()).unwrap_or(false);
    let version = body.get("version").and_then(|v| v.as_str()).unwrap_or("?");
    let server_status = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    println!("npurun serve at http://{addr}");
    println!("  status:    {server_status}");
    println!("  model:     {model}");
    println!("  uptime:    {}", format_uptime(uptime));
    println!(
        "  auth:      {}",
        if auth_on { "bearer-token" } else { "none" }
    );
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

/// Inspect and report the local NPU hardware setup.
///
/// Reads SoC name from `Win32_Processor`, looks for a Qualcomm Hexagon
/// NPU under `Get-PnpDevice`, infers the supported Hexagon architecture
/// from the QAIRT SDK directory layout (`lib/hexagon-vNN/`), and pairs
/// these with the QAIRT SDK + libGenie versions already shown by
/// [`print_versions`].
///
/// Reports facts; does not gate on them. The differentiation is exactly
/// this: AnythingLLM's QNN engine string-matches `Snapdragon(R) X Elite`
/// and refuses to start on X Plus / X 10-core machines (their issues
/// #2962 and #5129). npurun calls libGenie regardless and lets the SDK
/// either succeed or report a real failure mode.
fn show_hardware() -> Result<()> {
    let soc =
        detect_soc().unwrap_or_else(|| "(unknown — Win32_Processor probe failed)".to_string());
    let npu = detect_npu_pnp().unwrap_or_else(|| "(none reported by Get-PnpDevice)".to_string());
    let npu_driver = detect_npu_driver();
    let qairt_root = env::var("QNN_SDK_ROOT").ok();
    let hexagon_arch = qairt_root
        .as_ref()
        .and_then(|r| detect_hexagon_arch(Path::new(r)))
        .unwrap_or_else(|| "(QAIRT SDK layout unrecognised)".to_string());

    let qairt_version = qairt_root.as_ref().and_then(|root| {
        let sdk_yaml = Path::new(root).join("sdk.yaml");
        std::fs::read_to_string(&sdk_yaml).ok().and_then(|text| {
            text.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("version:")
                    .map(|rest| rest.trim().to_string())
            })
        })
    });
    let v = qnn::api_version();

    println!("SoC:              {soc}");
    println!("NPU:              {npu}");
    match npu_driver {
        Some((ver, Some(date))) => println!("NPU driver:       {ver}  ({date})"),
        Some((ver, None)) => println!("NPU driver:       {ver}"),
        None => println!(
            "NPU driver:       (Get-PnpDeviceProperty probe failed — check Device Manager)"
        ),
    }
    println!("Hexagon arch:     {hexagon_arch}");
    match (qairt_version, qairt_root.as_ref()) {
        (Some(ver), Some(root)) => println!("QAIRT SDK:        {ver}  ({root})"),
        (None, Some(root)) => println!("QAIRT SDK:        (sdk.yaml not parseable; root: {root})"),
        _ => println!("QAIRT SDK:        (QNN_SDK_ROOT not set)"),
    }
    println!("libGenie:         {}.{}.{}", v.major, v.minor, v.patch);
    println!();
    println!("Status:           Genie API loaded; npurun does not gate on SoC strings.");
    println!("                  If a model fails to load on this hardware, that is a");
    println!("                  real failure and worth filing as an issue with this");
    println!("                  output attached.");
    Ok(())
}

/// Probe `Win32_Processor` via PowerShell for the SoC marketing name.
/// Returns `None` if PowerShell or the WMI query fails — we don't
/// install a hard dependency on this for `show_hardware` to be useful.
fn detect_soc() -> Option<String> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name).Trim()",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Probe `Get-PnpDevice` for a Qualcomm Hexagon NPU. The friendly name
/// varies across SoC families ("Qualcomm AI Engine", "NPU Compute
/// Accelerator Device", etc.), so we filter by manufacturer + class
/// instead of name-matching.
fn detect_npu_pnp() -> Option<String> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-PnpDevice -Status OK | Where-Object { $_.Manufacturer -match 'Qualcomm' -and ($_.Class -eq 'Compute' -or $_.FriendlyName -match 'NPU|Hexagon|AI Engine') } | Select-Object -First 1 -ExpandProperty FriendlyName",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Probe `Get-PnpDeviceProperty` for the Qualcomm NPU's user-mode driver
/// version and date. Two machines on the same SoC + QAIRT SDK can still
/// fail differently if the HTP driver bundle (shipped by OEM / Windows
/// Update) diverges, and `Could not create context from binary` is the
/// classic symptom — see issue #12.
fn detect_npu_driver() -> Option<(String, Option<String>)> {
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            r#"$d = Get-PnpDevice -Status OK | Where-Object { $_.Manufacturer -match 'Qualcomm' -and ($_.Class -eq 'Compute' -or $_.FriendlyName -match 'NPU|Hexagon|AI Engine') } | Select-Object -First 1; if ($d) { $ver = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName 'DEVPKEY_Device_DriverVersion').Data; $date = (Get-PnpDeviceProperty -InstanceId $d.InstanceId -KeyName 'DEVPKEY_Device_DriverDate').Data; if ($date) { $ds = $date.ToString('yyyy-MM-dd') } else { $ds = '' }; "$ver|$ds" }"#,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let (ver, date) = s.split_once('|')?;
    let ver = ver.trim();
    let date = date.trim();
    if ver.is_empty() {
        return None;
    }
    let date = if date.is_empty() {
        None
    } else {
        Some(date.to_string())
    };
    Some((ver.to_string(), date))
}

/// Walk `<QAIRT>/lib/` looking for a `hexagon-vNN/` directory. The SDK
/// ships exactly one such directory per supported HTP architecture
/// (`hexagon-v73` for QAIRT 2.45 → X1E / X Plus / X 10-core; later SDKs
/// add `v75` / `v79` for X2-class silicon).
fn detect_hexagon_arch(qairt_root: &Path) -> Option<String> {
    let lib_dir = qairt_root.join("lib");
    let entries = std::fs::read_dir(&lib_dir).ok()?;
    let mut found: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(rest) = name.strip_prefix("hexagon-v") {
            if rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
                found.push(format!("hexagon-v{rest}"));
            }
        }
    }
    found.sort();
    if found.is_empty() {
        None
    } else {
        Some(found.join(", "))
    }
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
    ctx: Option<u32>,
    csv_path: Option<&Path>,
    duration_secs: Option<u64>,
) -> Result<()> {
    let dir = resolve_model_dir(model)?;
    if let Some(requested) = ctx {
        let tiers = Engine::available_ctx_tiers(&dir);
        if !tiers.is_empty() && !tiers.contains(&requested) {
            return Err(anyhow!(
                "context tier {requested} not available in this bundle; available tiers: {}",
                tiers
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let cfg = EngineConfig {
        model_dir: dir,
        ctx,
        ..Default::default()
    };
    let load_started = Instant::now();
    let engine = Engine::load(cfg)?;
    let load_elapsed = load_started.elapsed();
    let resolved_ctx = ctx.unwrap_or(engine.manifest().context);
    eprintln!(
        "==  npurun bench: {} ({}, {:?}, ctx {})  ==",
        engine.manifest().name,
        engine.manifest().arch,
        engine.manifest().quant,
        resolved_ctx,
    );
    eprintln!("[bundle loaded in {load_elapsed:.2?}]");

    let mut csv_writer = match csv_path {
        Some(path) => Some(open_bench_csv(path)?),
        None => None,
    };

    let prompts: Vec<&str> = match custom_prompt {
        Some(p) => vec![p],
        None => BENCH_PROMPTS.to_vec(),
    };
    let stress = duration_secs.is_some();
    let stress_window = duration_secs.map(Duration::from_secs);
    if stress {
        eprintln!(
            "[stress mode: cycling prompts for {}s]\n",
            duration_secs.unwrap()
        );
    } else {
        let total_runs = prompts.len() * repeats.max(1);
        eprintln!("[running {total_runs} queries]\n");
    }

    let model_name = engine.manifest().name.clone();
    let mut runs: Vec<RunStat> = Vec::new();
    let mut idx = 0usize;
    let stress_started = Instant::now();
    let mut repeat_idx = 0usize;
    'outer: loop {
        for prompt in &prompts {
            // Stress-mode budget check before each query so we always
            // finish the in-flight one but don't start a new one past
            // the wall.
            if let Some(window) = stress_window {
                if stress_started.elapsed() >= window {
                    break 'outer;
                }
            }
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
            engine.reset_dialog()?;
            let ttft = first_token_at.unwrap_or(total);
            let tokens = approx_token_count(&output);
            let gen_time = total.saturating_sub(ttft);
            let tps_post = if gen_time.as_secs_f64() > 0.0 {
                tokens as f64 / gen_time.as_secs_f64()
            } else {
                0.0
            };
            // Stress mode shows a single line per query so 5+ minute
            // runs don't drown the terminal in full responses.
            if stress {
                let elapsed_s = stress_started.elapsed().as_secs_f64();
                println!(
                    "  [t+{elapsed_s:6.1}s  q{idx:4}] tokens={tokens:3} ttft={:5} ms tps={tps_post:5.1}",
                    ttft.as_millis()
                );
            } else {
                println!("--- query {idx} ---");
                println!("    prompt: {prompt}");
                println!("    response ({tokens} approx tokens): {}", output.trim());
                println!("    total: {total:.2?}   ttft: {ttft:.2?}   gen: {gen_time:.2?}   tok/s post-ttft: {tps_post:.1}");
                println!();
            }
            runs.push(RunStat {
                total,
                ttft,
                gen_time,
                tokens,
            });
            if let Some(w) = csv_writer.as_mut() {
                write_bench_csv_row(
                    w,
                    &model_name,
                    prompt,
                    repeat_idx + 1,
                    resolved_ctx,
                    ttft,
                    total,
                    gen_time,
                    tokens,
                    tps_post,
                )
                .with_context(|| format!("writing CSV row to {}", csv_path.unwrap().display()))?;
            }
        }
        repeat_idx += 1;
        if !stress && repeat_idx >= repeats.max(1) {
            break;
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

    // Stress-mode extended stats: percentiles, std-dev, and a
    // first-half-vs-second-half degradation percent. Degradation is the
    // single number that catches thermal throttling: if the second half
    // of a 5+ minute run is materially slower than the first half, the
    // chip is throttling under sustained load.
    if stress {
        let mut tps_samples: Vec<f64> = warm
            .iter()
            .map(|r| {
                if r.gen_time.as_secs_f64() > 0.0 {
                    r.tokens as f64 / r.gen_time.as_secs_f64()
                } else {
                    0.0
                }
            })
            .collect();
        tps_samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |p: f64| -> f64 {
            let idx = ((p / 100.0) * (tps_samples.len() as f64 - 1.0)).round() as usize;
            tps_samples[idx.min(tps_samples.len() - 1)]
        };
        let mean: f64 = tps_samples.iter().sum::<f64>() / tps_samples.len() as f64;
        let variance: f64 = tps_samples
            .iter()
            .map(|x| (x - mean).powi(2))
            .sum::<f64>()
            / tps_samples.len() as f64;
        let stdev = variance.sqrt();
        let half = warm.len() / 2;
        let first_half_tps = if half > 0 {
            let toks: usize = warm.iter().take(half).map(|r| r.tokens).sum();
            let secs: f64 = warm
                .iter()
                .take(half)
                .map(|r| r.gen_time.as_secs_f64())
                .sum();
            if secs > 0.0 { toks as f64 / secs } else { 0.0 }
        } else {
            0.0
        };
        let second_half_tps = if warm.len() - half > 0 {
            let toks: usize = warm.iter().skip(half).map(|r| r.tokens).sum();
            let secs: f64 = warm
                .iter()
                .skip(half)
                .map(|r| r.gen_time.as_secs_f64())
                .sum();
            if secs > 0.0 { toks as f64 / secs } else { 0.0 }
        } else {
            0.0
        };
        let degradation_pct = if first_half_tps > 0.0 {
            ((first_half_tps - second_half_tps) / first_half_tps) * 100.0
        } else {
            0.0
        };
        println!();
        println!("==  stress stats  ==");
        println!(
            "    queries completed:        {} (in {:.1}s)",
            warm.len(),
            stress_started.elapsed().as_secs_f64()
        );
        println!("    tok/s post-ttft min:      {:.1}", tps_samples[0]);
        println!("    tok/s post-ttft p50:      {:.1}", pct(50.0));
        println!("    tok/s post-ttft p90:      {:.1}", pct(90.0));
        println!(
            "    tok/s post-ttft max:      {:.1}",
            tps_samples[tps_samples.len() - 1]
        );
        println!("    tok/s post-ttft stdev:    {stdev:.2}");
        println!(
            "    first-half tps:           {first_half_tps:.1}  -> second-half tps: {second_half_tps:.1}  (degradation {degradation_pct:+.1}%)"
        );
        if degradation_pct > 5.0 {
            println!(
                "    NOTE: >5% degradation between halves likely indicates thermal throttling."
            );
        }
    }
    Ok(())
}

#[derive(Debug)]
struct RunStat {
    total: Duration,
    ttft: Duration,
    gen_time: Duration,
    tokens: usize,
}

const BENCH_CSV_HEADER: &str =
    "model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft";

/// Open the bench CSV at `path`, returning a buffered writer ready for
/// row appends. Writes the header row only when the file did not exist
/// (so re-runs against the same path append cleanly across versions).
/// Errors with a clear message if the parent directory is missing.
fn open_bench_csv(path: &Path) -> Result<std::io::BufWriter<std::fs::File>> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(anyhow!(
                "CSV parent directory does not exist: {}",
                parent.display()
            ));
        }
    }
    let needs_header = !path.exists();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening CSV {}", path.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    if needs_header {
        writeln!(writer, "{BENCH_CSV_HEADER}")?;
    }
    Ok(writer)
}

#[allow(clippy::too_many_arguments)]
fn write_bench_csv_row(
    writer: &mut std::io::BufWriter<std::fs::File>,
    model: &str,
    prompt: &str,
    repeat: usize,
    ctx: u32,
    ttft: Duration,
    total: Duration,
    gen_time: Duration,
    tokens: usize,
    tps_post: f64,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "{model},{prompt_field},{repeat},{ctx},{ttft_ms:.3},{total_ms:.3},{gen_ms:.3},{tokens},{tps_post:.3}",
        model = csv_field(model),
        prompt_field = csv_field(prompt),
        ttft_ms = ttft.as_secs_f64() * 1000.0,
        total_ms = total.as_secs_f64() * 1000.0,
        gen_ms = gen_time.as_secs_f64() * 1000.0,
    )?;
    writer.flush()
}

/// Quote a CSV field per RFC 4180 if it contains a comma, quote, or
/// newline; otherwise emit it bare.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

/// Strip an Ollama-style `:tag` suffix so `phi-3.5-mini:latest` compares
/// equal to `phi-3.5-mini` against a server's loaded-model name.
fn bare_model_name(name: &str) -> &str {
    name.split(':').next().unwrap_or(name)
}

/// Dispatch a prompt to a running `npurun serve` over HTTP, streaming the
/// SSE response to stdout. Validates the loaded model first via
/// `/healthz` and aborts cleanly on mismatch (preferred per the
/// follow-up briefing) rather than letting the server return whatever
/// it has loaded.
async fn run_remote(addr: &str, model: &str, prompt: &str, auth_token: Option<&str>) -> Result<()> {
    if prompt.is_empty() {
        return Err(anyhow!("prompt is empty; provide one as trailing argument"));
    }
    let bare = bare_model_name(model);
    let client = reqwest::Client::builder()
        .build()
        .context("building HTTP client")?;

    // 1. Validate the server is up and serving the requested model.
    let health_url = format!("http://{addr}/healthz");
    let mut req = client.get(&health_url).timeout(Duration::from_secs(2));
    if let Some(tok) = auth_token {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await.map_err(|e| {
        anyhow!(
            "no npurun serve responding at {addr} ({e}); either start one with `npurun serve` or drop --addr to load locally"
        )
    })?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "server at {addr} returned HTTP {} for /healthz",
            resp.status()
        ));
    }
    let body_bytes = resp
        .bytes()
        .await
        .with_context(|| format!("reading /healthz body from {addr}"))?;
    let body: serde_json::Value = serde_json::from_slice(&body_bytes)
        .with_context(|| format!("parsing /healthz body from {addr}"))?;
    let loaded = body
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("server at {addr} has no model loaded; cannot dispatch run"))?;
    if bare_model_name(loaded) != bare {
        return Err(anyhow!(
            "server at {addr} has {loaded:?} loaded, but you asked to run {model:?}. \
             Either restart serve with --model {bare}, point --addr at the right server, \
             or drop --addr to load locally."
        ));
    }

    // 2. Stream the chat completion.
    let chat_url = format!("http://{addr}/v1/chat/completions");
    let payload = serde_json::json!({
        "model": bare,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true,
    });
    let body = serde_json::to_string(&payload).context("serializing chat request body")?;
    let mut req = client
        .post(&chat_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body);
    if let Some(tok) = auth_token {
        req = req.bearer_auth(tok);
    }
    let started = Instant::now();
    let resp = req
        .send()
        .await
        .with_context(|| format!("POST {chat_url}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Per the briefing: do not retry on 429 — defeats the point of
        // bypassing cold-load.
        return Err(anyhow!(
            "server at {addr} is busy (HTTP 429); another request holds the inference permit"
        ));
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("server at {addr} returned HTTP {status}: {text}"));
    }

    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunks_seen = 0usize;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading SSE chunk")?;
        buf.extend_from_slice(&chunk);
        // Each SSE event ends with a blank line (`\n\n`). Consume
        // complete events out of the buffer; leave the trailing partial
        // event behind for the next iteration.
        while let Some((end, term_len)) = find_event_terminator(&buf) {
            let raw = buf[..end].to_vec();
            buf.drain(..end + term_len);
            if let Some(content) = extract_sse_content(&raw) {
                if content == "[DONE]" {
                    // Final event — stream is done.
                    break;
                }
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(piece) = parsed
                        .get("choices")
                        .and_then(|c| c.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        if !piece.is_empty() {
                            print!("{piece}");
                            std::io::stdout().flush().ok();
                            chunks_seen += 1;
                        }
                    }
                }
            }
        }
    }
    let elapsed = started.elapsed();
    println!();
    eprintln!(
        "[remote {addr}: {chunks_seen} chunks in {elapsed:.2?}; ~{:.1} chunks/s]",
        chunks_seen as f64 / elapsed.as_secs_f64()
    );
    Ok(())
}

/// Find the index and length of the next SSE event terminator (`\n\n`
/// or `\r\n\r\n`) in `buf`. Returns the start index plus the
/// terminator's byte length, or `None` if no complete event is yet
/// buffered.
fn find_event_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n");
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(a), Some(b)) if a <= b => Some((a, 2)),
        (Some(_), Some(b)) => Some((b, 4)),
        (Some(a), None) => Some((a, 2)),
        (None, Some(b)) => Some((b, 4)),
        (None, None) => None,
    }
}

/// Extract the concatenated `data:` payload from a single SSE event.
/// Multi-line `data:` runs are joined with `\n` per the SSE spec.
/// Returns `None` if the event has no `data:` lines (comments, retry
/// directives, etc.).
fn extract_sse_content(event: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(event).ok()?;
    let mut data = String::new();
    let mut any = false;
    for line in text.split(['\n', '\r']) {
        if let Some(rest) = line.strip_prefix("data:") {
            if any {
                data.push('\n');
            }
            data.push_str(rest.trim_start_matches(' '));
            any = true;
        }
    }
    if any {
        Some(data)
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_passes_simple_strings_through() {
        assert_eq!(csv_field("hello"), "hello");
        assert_eq!(csv_field("phi-3.5-mini"), "phi-3.5-mini");
    }

    #[test]
    fn csv_field_quotes_commas_quotes_newlines() {
        assert_eq!(csv_field("a, b"), "\"a, b\"");
        assert_eq!(csv_field("she said \"hi\""), "\"she said \"\"hi\"\"\"");
        assert_eq!(csv_field("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn csv_writer_writes_header_then_row() {
        let tmp =
            std::env::temp_dir().join(format!("npurun-bench-csv-test-{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&tmp);

        let mut w = open_bench_csv(&tmp).expect("open csv");
        write_bench_csv_row(
            &mut w,
            "phi-3.5-mini",
            "Hello, world",
            1,
            1024,
            Duration::from_millis(120),
            Duration::from_millis(800),
            Duration::from_millis(680),
            42,
            61.7,
        )
        .expect("write row");
        drop(w);

        let body = std::fs::read_to_string(&tmp).expect("read csv");
        let mut lines = body.lines();
        assert_eq!(lines.next(), Some(BENCH_CSV_HEADER));
        let row = lines.next().expect("data row");
        assert!(row
            .starts_with("phi-3.5-mini,\"Hello, world\",1,1024,120.000,800.000,680.000,42,61.700"));
        assert_eq!(lines.next(), None);

        // Re-open: must NOT emit a second header (caller is appending).
        let mut w2 = open_bench_csv(&tmp).expect("reopen csv");
        write_bench_csv_row(
            &mut w2,
            "phi-3.5-mini",
            "second",
            2,
            1024,
            Duration::from_millis(50),
            Duration::from_millis(200),
            Duration::from_millis(150),
            10,
            66.6,
        )
        .expect("append row");
        drop(w2);
        let body2 = std::fs::read_to_string(&tmp).expect("read csv after append");
        let header_count = body2.lines().filter(|l| *l == BENCH_CSV_HEADER).count();
        assert_eq!(header_count, 1, "header must only appear once");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn sse_terminator_finds_lf_lf() {
        let buf = b"data: hi\n\nrest";
        let (end, len) = find_event_terminator(buf).expect("found");
        assert_eq!(&buf[..end], b"data: hi");
        assert_eq!(len, 2);
    }

    #[test]
    fn sse_terminator_finds_crlf_crlf() {
        let buf = b"data: hi\r\n\r\nrest";
        let (end, len) = find_event_terminator(buf).expect("found");
        assert_eq!(&buf[..end], b"data: hi");
        assert_eq!(len, 4);
    }

    #[test]
    fn sse_terminator_returns_none_for_partial_event() {
        assert!(find_event_terminator(b"data: hi\n").is_none());
    }

    #[test]
    fn sse_extract_handles_chat_chunk() {
        let evt = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}";
        let payload = extract_sse_content(evt).expect("payload");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["choices"][0]["delta"]["content"].as_str(), Some("hi"));
    }

    #[test]
    fn sse_extract_returns_done_marker() {
        assert_eq!(
            extract_sse_content(b"data: [DONE]").as_deref(),
            Some("[DONE]")
        );
    }

    #[test]
    fn sse_extract_skips_comment_only_event() {
        assert_eq!(extract_sse_content(b": keep-alive"), None);
    }

    #[test]
    fn detect_hexagon_arch_finds_compiled_archs() {
        let tmp = std::env::temp_dir().join(format!("npurun-hexagon-test-{}", std::process::id()));
        let lib = tmp.join("lib");
        std::fs::create_dir_all(lib.join("hexagon-v73").join("unsigned")).unwrap();
        std::fs::create_dir_all(lib.join("hexagon-v79").join("unsigned")).unwrap();
        std::fs::create_dir_all(lib.join("aarch64-windows-msvc")).unwrap();

        let arch = detect_hexagon_arch(&tmp).expect("found");
        assert_eq!(arch, "hexagon-v73, hexagon-v79");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detect_hexagon_arch_returns_none_when_layout_missing() {
        let tmp = std::env::temp_dir().join(format!("npurun-hexagon-empty-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("lib")).unwrap();
        assert!(detect_hexagon_arch(&tmp).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn bare_model_name_strips_tag() {
        assert_eq!(bare_model_name("phi-3.5-mini:latest"), "phi-3.5-mini");
        assert_eq!(bare_model_name("phi-3.5-mini"), "phi-3.5-mini");
    }

    #[test]
    fn csv_writer_errors_on_missing_parent_dir() {
        let bogus = std::env::temp_dir()
            .join("npurun-bench-csv-no-such-dir-9zA3pq")
            .join("out.csv");
        let err = open_bench_csv(&bogus).expect_err("must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("CSV parent directory does not exist"),
            "unexpected error message: {msg}"
        );
    }
}

//! Phase 1 NPU smoke test, driven from native Rust.
//!
//! Loads the Qwen 2.5 7B Genie bundle directly via the safe `qnn::genie`
//! wrapper (no shell-out to `genie-t2t-run.exe`) and runs a single
//! generation. Equivalent in behaviour to `scripts/genie-run.ps1` but with
//! the full lifecycle (config load -> dialog create -> query -> drop)
//! happening in this process. This is the milestone that proves the
//! Phase 1 Rust bindings work end-to-end.
//!
//! Usage (after `scripts\dev-shell.bat` sets up vcvarsall):
//!
//! ```text
//! cargo run --release -p qnn --example qwen-genie -- "Tell me a joke."
//! ```
//!
//! Requires QNN_SDK_ROOT to be set, plus the QAIRT runtime DLLs and the
//! Hexagon stubs to be reachable. The example arranges PATH and
//! ADSP_LIBRARY_PATH internally so it works without external setup.

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use qnn::genie::{api_version, Dialog, SentenceCode};

const DEFAULT_BUNDLE: &str = r"C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite";
const DEFAULT_PROMPT: &str = "Tell me a one-line joke about Snapdragon laptops.";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,qnn=debug")),
        )
        .init();

    let prompt = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let prompt = if prompt.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        prompt
    };

    let bundle = env::var("NPURUN_BUNDLE").unwrap_or_else(|_| DEFAULT_BUNDLE.to_string());
    let bundle = PathBuf::from(bundle);
    let config = bundle.join("genie_config.json");

    let qairt = env::var("QNN_SDK_ROOT")
        .map_err(|_| anyhow::anyhow!("QNN_SDK_ROOT not set; run inside scripts\\dev-shell.bat"))?;
    setup_qairt_environment(&qairt)?;

    let v = api_version();
    println!("libGenie {}.{}.{}", v.major, v.minor, v.patch);
    println!("bundle:  {}", bundle.display());
    println!("config:  {}", config.display());
    println!("prompt:  {prompt}");
    println!();

    let load_started = Instant::now();
    let dialog = Dialog::from_config_file(&config)?;
    let load_elapsed = load_started.elapsed();
    println!("[loaded bundle in {load_elapsed:.2?}]");
    println!();

    let prompt_wrapped = format!(
        "<|im_start|>system\nYou are a helpful assistant.<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
    );

    let infer_started = Instant::now();
    let mut sentence_codes_seen: Vec<SentenceCode> = Vec::new();
    print!("response: ");
    use std::io::Write;
    std::io::stdout().flush().ok();

    dialog.query_streaming(&prompt_wrapped, |chunk, code| {
        sentence_codes_seen.push(code);
        print!("{chunk}");
        std::io::stdout().flush().ok();
    })?;
    println!();
    let infer_elapsed = infer_started.elapsed();
    println!();
    println!("[generation took {infer_elapsed:.2?}]");
    println!("[sentence codes seen: {sentence_codes_seen:?}]");

    // Dialog::drop runs here, freeing 4.6 GB of NPU shared memory.
    Ok(())
}

/// Add QAIRT bin/lib dirs to PATH and set ADSP_LIBRARY_PATH so the runtime
/// can find Hexagon stubs. Mirrors what scripts/genie-run.ps1 does.
fn setup_qairt_environment(qairt: &str) -> anyhow::Result<()> {
    let qairt = PathBuf::from(qairt);
    let bin = qairt.join("bin").join("aarch64-windows-msvc");
    let lib = qairt.join("lib").join("aarch64-windows-msvc");
    let adsp = qairt.join("lib").join("hexagon-v73").join("unsigned");

    for required in [&bin, &lib, &adsp] {
        if !required.exists() {
            anyhow::bail!(
                "QAIRT directory missing: {}. Set QNN_SDK_ROOT to a valid QAIRT 2.44+ install.",
                required.display()
            );
        }
    }

    // Prepend bin and lib to PATH (so Genie.dll and its deps are found).
    let path_var = env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::new();
    new_path.push(&bin);
    new_path.push(";");
    new_path.push(&lib);
    new_path.push(";");
    new_path.push(&path_var);
    env::set_var("PATH", new_path);

    env::set_var("ADSP_LIBRARY_PATH", &adsp);
    env::set_var("QAIRT_HOME", &qairt);
    Ok(())
}

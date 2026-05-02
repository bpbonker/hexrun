//! One long Qwen3-4B generation for NPU-utilization observation.
//!
//! Asks the model to write ~500 tokens of output so the decode loop runs
//! for 30+ seconds — long enough to watch Task Manager / Performance /
//! NPU and see whether decode pegs the device or sits idle. Reports
//! per-second token rate at the end.
//!
//! Run inside scripts\dev-shell.bat:
//!   cargo run --release -p qnn --example qwen3-long

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use qnn::genie::Dialog;

const DEFAULT_BUNDLE: &str = r"C:\AAA\Personal\AI\models\qwen3-4b-instruct-2507\bundle\qwen3_4b_instruct_2507-genie-w4a16-qualcomm_snapdragon_x_elite";

const PROMPT: &str = "<|im_start|>system\nYou are a helpful assistant. Be detailed and thorough.<|im_end|>\n<|im_start|>user\nWrite a detailed 500-word essay on the history and design of RISC processors, covering ARM, MIPS, RISC-V, and the trade-offs versus CISC. Include specific examples and dates.<|im_end|>\n<|im_start|>assistant\n";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,qnn=info")),
        )
        .init();

    let bundle = env::var("NPURUN_BUNDLE").unwrap_or_else(|_| DEFAULT_BUNDLE.to_string());
    let bundle = PathBuf::from(bundle);
    let config = bundle.join("genie_config.json");

    let qairt = env::var("QNN_SDK_ROOT")
        .map_err(|_| anyhow::anyhow!("QNN_SDK_ROOT not set; run inside scripts\\dev-shell.bat"))?;
    setup_qairt_environment(&qairt)?;

    println!("==  qwen3-4b long-generation NPU watch  ==");
    println!("bundle: {}", bundle.display());
    println!("Watch Task Manager -> Performance -> NPU during the run.");
    println!();

    let load_started = Instant::now();
    let dialog = Dialog::from_config_file(&config)?;
    println!("[bundle loaded in {:.2?}]", load_started.elapsed());
    println!();

    println!("[generating; output streamed below]");
    let started = Instant::now();
    let mut first_token_at: Option<Duration> = None;
    let mut chunks = 0usize;
    let mut output = String::new();
    let mut last_print = Instant::now();
    dialog.query_streaming(PROMPT, |chunk, _code| {
        if first_token_at.is_none() && !chunk.is_empty() {
            first_token_at = Some(started.elapsed());
        }
        chunks += 1;
        output.push_str(chunk);
        // Print a heartbeat every second so the screen shows progress
        // and you can correlate with Task Manager.
        if last_print.elapsed() >= Duration::from_secs(1) {
            let elapsed = started.elapsed().as_secs_f64();
            let words = output.split_whitespace().count();
            let tps = (words as f64 * 1.3) / elapsed;
            eprintln!(
                "  [t+{elapsed:6.1}s  chunks={chunks:5}  approx_tokens={:5}  tps={tps:5.1}]",
                (words as f64 * 1.3) as usize
            );
            last_print = Instant::now();
        }
    })?;
    let total = started.elapsed();
    let ttft = first_token_at.unwrap_or(total);
    let words = output.split_whitespace().count();
    let approx_tokens = ((words as f64) * 1.3).round() as usize;
    let gen_time = total.saturating_sub(ttft);
    let tps_avg = approx_tokens as f64 / total.as_secs_f64();
    let tps_post_first = if gen_time.as_secs_f64() > 0.0 {
        approx_tokens as f64 / gen_time.as_secs_f64()
    } else {
        0.0
    };

    println!();
    println!("---  output ({approx_tokens} approx tokens, {chunks} chunks)  ---");
    println!("{}", output.trim());
    println!();
    println!("---  timing  ---");
    println!("    total:       {total:.2?}");
    println!("    ttft:        {ttft:.2?}");
    println!("    gen time:    {gen_time:.2?}");
    println!("    tok/s avg:   {tps_avg:.2}");
    println!("    tok/s post:  {tps_post_first:.2}");

    Ok(())
}

fn setup_qairt_environment(qairt: &str) -> anyhow::Result<()> {
    let qairt = PathBuf::from(qairt);
    let bin = qairt.join("bin").join("aarch64-windows-msvc");
    let lib = qairt.join("lib").join("aarch64-windows-msvc");
    let adsp = qairt.join("lib").join("hexagon-v73").join("unsigned");

    for required in [&bin, &lib, &adsp] {
        if !required.exists() {
            anyhow::bail!("QAIRT directory missing: {}", required.display());
        }
    }

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

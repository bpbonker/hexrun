//! Warm-query benchmark for Phi 3.5 Mini Instruct (NPU bundle from
//! `qualcomm/Phi-3.5-mini-instruct` on Hugging Face).
//!
//! Same methodology as `qwen-bench`: load once, run several queries,
//! measure cold load + per-query latency + steady-state tok/s. Differs
//! only in the bundle path and the chat-template wrapping.
//!
//! Phi 3 chat template:
//!     <|system|>...<|end|>
//!     <|user|>...<|end|>
//!     <|assistant|>

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use qnn::genie::Dialog;

const DEFAULT_BUNDLE: &str = r"C:\AAA\Personal\AI\models\phi-3.5-mini\bundle\phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite";

const PROMPTS: &[&str] = &[
    "Write a one-line joke about Snapdragon laptops.",
    "Briefly explain why an NPU is more energy-efficient than a CPU for matrix multiplication.",
    "List three reasons running language models locally on a laptop is useful.",
    "What is 17 multiplied by 23? Just the number.",
];

fn wrap_prompt(user: &str) -> String {
    format!(
        "<|system|>\nYou are a concise assistant. Answer in 1-2 sentences.<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n"
    )
}

fn approx_token_count(s: &str) -> usize {
    let words = s.split_whitespace().count();
    ((words as f64) * 1.3).round() as usize
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,qnn=info")),
        )
        .init();

    let bundle = env::var("HEXRUN_BUNDLE").unwrap_or_else(|_| DEFAULT_BUNDLE.to_string());
    let bundle = PathBuf::from(bundle);
    let config = bundle.join("genie_config.json");

    let qairt = env::var("QNN_SDK_ROOT")
        .map_err(|_| anyhow::anyhow!("QNN_SDK_ROOT not set; run inside scripts\\dev-shell.bat"))?;
    setup_qairt_environment(&qairt)?;

    println!("==  hexrun warm-query benchmark: Phi 3.5 Mini  ==");
    println!("bundle: {}", bundle.display());
    println!();

    let load_started = Instant::now();
    let dialog = Dialog::from_config_file(&config)?;
    let load_elapsed = load_started.elapsed();
    println!("[bundle loaded in {load_elapsed:.2?}]");
    println!();

    let mut runs: Vec<RunStat> = Vec::new();
    for (i, prompt) in PROMPTS.iter().enumerate() {
        let wrapped = wrap_prompt(prompt);
        let started = Instant::now();
        let mut output = String::new();
        let mut first_token_at: Option<Duration> = None;
        dialog.query_streaming(&wrapped, |chunk, _code| {
            if first_token_at.is_none() && !chunk.is_empty() {
                first_token_at = Some(started.elapsed());
            }
            output.push_str(chunk);
        })?;
        let total = started.elapsed();
        let ttft = first_token_at.unwrap_or(total);
        let tokens = approx_token_count(&output);
        let gen_time = total.saturating_sub(ttft);
        let tps_avg = tokens as f64 / total.as_secs_f64();
        let tps_post_first = if gen_time.as_secs_f64() > 0.0 {
            tokens as f64 / gen_time.as_secs_f64()
        } else {
            0.0
        };
        println!("--- query {} ---", i + 1);
        println!("    prompt: {prompt}");
        println!("    response ({tokens} approx tokens): {}", output.trim());
        println!("    total: {total:.2?}   ttft: {ttft:.2?}   gen: {gen_time:.2?}");
        println!("    tok/s (incl. ttft): {tps_avg:.1}   tok/s (after ttft): {tps_post_first:.1}");
        println!();
        runs.push(RunStat {
            total,
            ttft,
            gen_time,
            tokens,
        });
    }

    let warm: Vec<&RunStat> = runs.iter().skip(1).collect();
    if !warm.is_empty() {
        let n = warm.len() as u32;
        let avg = |f: fn(&RunStat) -> Duration| -> Duration {
            warm.iter().map(|r| f(r)).sum::<Duration>() / n
        };
        let total_tokens: usize = warm.iter().map(|r| r.tokens).sum();
        let total_secs: f64 = warm.iter().map(|r| r.total.as_secs_f64()).sum();
        let total_gen_secs: f64 = warm.iter().map(|r| r.gen_time.as_secs_f64()).sum();
        println!("==  warm summary (skipping first query) ==");
        println!("    queries:                    {}", warm.len());
        println!("    avg total per query:        {:.2?}", avg(|r| r.total));
        println!("    avg time-to-first-token:    {:.2?}", avg(|r| r.ttft));
        println!(
            "    avg generation time:        {:.2?}",
            avg(|r| r.gen_time)
        );
        println!(
            "    aggregate tok/s (incl ttft): {:.1}",
            total_tokens as f64 / total_secs
        );
        println!(
            "    aggregate tok/s (post ttft): {:.1}",
            total_tokens as f64 / total_gen_secs
        );
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

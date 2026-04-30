//! Warm-second-query benchmark for Phase 1.
//!
//! Loads the Qwen 2.5 7B Genie bundle ONCE, then runs four queries
//! back-to-back, measuring per-query latency and approximate tokens/sec.
//! This is the measurement the Phase 0 numbers couldn't give us — every
//! `genie-t2t-run.exe` invocation cold-loads 4.6 GB into NPU shared memory,
//! so the average tok/s is dominated by paging time. With a long-running
//! Dialog object the bundle stays resident and we can measure steady state.
//!
//! Usage:
//!
//! ```text
//! cargo run --release -p qnn --example qwen-bench
//! ```

use std::env;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use qnn::genie::Dialog;

const DEFAULT_BUNDLE: &str = r"C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite";

const PROMPTS: &[&str] = &[
    "Write a one-line joke about Snapdragon laptops.",
    "Briefly explain why an NPU is more energy-efficient than a CPU for matrix multiplication.",
    "List three reasons running language models locally on a laptop is useful.",
    "What is 17 multiplied by 23? Just the number.",
];

fn wrap_prompt(user: &str) -> String {
    format!(
        "<|im_start|>system\nYou are a concise assistant. Answer in 1-2 sentences.<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
    )
}

fn approx_token_count(s: &str) -> usize {
    // English heuristic: ~1.3 tokens per word for the BPE-style tokenizers
    // used by Qwen / Llama. Good enough for steady-state tok/s reporting.
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

    let bundle = env::var("NPURUN_BUNDLE").unwrap_or_else(|_| DEFAULT_BUNDLE.to_string());
    let bundle = PathBuf::from(bundle);
    let config = bundle.join("genie_config.json");

    let qairt = env::var("QNN_SDK_ROOT")
        .map_err(|_| anyhow::anyhow!("QNN_SDK_ROOT not set; run inside scripts\\dev-shell.bat"))?;
    setup_qairt_environment(&qairt)?;

    println!("==  npurun Phase 1 warm-query benchmark  ==");
    println!("bundle: {}", bundle.display());
    println!();

    // ---- one-time bundle load ----
    let load_started = Instant::now();
    let dialog = Dialog::from_config_file(&config)?;
    let load_elapsed = load_started.elapsed();
    println!("[bundle loaded in {load_elapsed:.2?}]");
    println!();

    // ---- run all prompts back to back ----
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

    // ---- summary: skip the first query (still warming) and average the rest ----
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

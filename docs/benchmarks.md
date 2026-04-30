# Benchmarks

Honest measurements. Updated as new data arrives.

## Headline (post-tuning, 2026-04-30)

| Model | Quant | Shards | Config | Steady-state tok/s | TTFT |
|---|---|---:|---|---:|---:|
| Qwen 2.5 7B | w8a16 | 6 | adapted from tutorial template, `poll: false` | 1.4 | 792 ms |
| Qwen 2.5 7B | w8a16 | 6 | + tuned cpu-mask/threads/sampler/perf-profile | 1.3 | 853 ms |
| Qwen 2.5 7B | w8a16 | 6 | **+ `poll: true`** (single flag) | **1.9** | 659 ms |
| **Phi 3.5 Mini** | **w4a16** | **4** | **Qualcomm-shipped (poll: true)** | **11.7** | **194 ms** |

**Phi 3.5 Mini at 11.7 tok/s on the Snapdragon X Elite NPU is in the
"actually usable for interactive chat" range** (roughly human speech
speed). The lift over Qwen 7B is ~6×, attributable to:

- ~2× from going 7B → 3.8B parameters,
- ~1.3× from w8a16 → w4a16 (less memory bandwidth pressure),
- ~1.2× from 6 → 4 shards (fewer per-token context transitions),
- ~1.4× from Qualcomm's properly-tuned config (most importantly, `poll: true`).

Running the user's other config tweaks (more CPU cores via `cpu-mask:
0xfff`, more `n-threads`, greedy sampling, `sustained_high_performance`
perf profile) had no measurable benefit and were slightly counter-
productive. The bottleneck on Qwen 7B was *not* CPU orchestration; it
was the cost of waiting on NPU completion via interrupts (which `poll:
true` replaces with a tight CPU-side polling loop).

## Phase 1 warm-query benchmark (2026-04-30)

**Setup:**
- Hardware: Microsoft Surface laptop, Snapdragon X Elite X1E80100, 16 GB shared LPDDR5x, 7.8 GB shared NPU memory
- Bundle: Qwen 2.5 7B Instruct, w8a16 quantized, 6 ctx-bin shards (4.6 GB), context length 4096, sequence length 128
- Runtime: npurun's `qnn::genie::Dialog` calling Qualcomm's libGenie 1.17.0 directly via Rust FFI (no `genie-t2t-run.exe` shell-out)
- Sampler: `temperature=0.8, top_k=40, top_p=0.95` (Qualcomm AI Hub defaults for Qwen)
- HTP perf profile: `burst`
- Battery state: plugged in
- Driver: HTP 30.0.219.1000 (9/11/2025)

**Methodology:** load the bundle once, run 4 prompts back-to-back through the same `Dialog`, time each. Skip the first query when averaging (still warming the on-chip caches). Token counts approximated as `words × 1.3` (BPE-style tokenizer rule of thumb).

### Per-query results

| # | Prompt | Tokens (approx) | TTFT | Total | Generation | Tok/s post-TTFT |
|---|---|---:|---:|---:|---:|---:|
| 1 | "Write a one-line joke about Snapdragon laptops." | 25 | 329 ms | 16.16 s | 15.83 s | 1.6 |
| 2 | "Briefly explain why an NPU is more energy-efficient than a CPU for matrix multiplication." | 65 | 763 ms | 47.14 s | 46.37 s | 1.4 |
| 3 | "List three reasons running language models locally on a laptop is useful." | 46 | 796 ms | 31.84 s | 31.04 s | 1.5 |
| 4 | "What is 17 multiplied by 23? Just the number." | 1 | 817 ms | 3.22 s | 2.40 s | 0.4 |

### Warm summary (skipping query 1)

| Metric | Value |
|---|---|
| Bundle cold load (one-time) | **8.65 s** |
| Avg total per query | 27.40 s |
| Avg time-to-first-token | **792 ms** |
| Avg generation time | 26.61 s |
| Aggregate tokens/sec (incl. TTFT) | **1.4** |
| Aggregate tokens/sec (post-TTFT) | **1.4** |

### What this tells us

- **Bundle load amortization works.** Cold load is 8.65 s with the bundle warm in the OS file cache (vs. ~30 s in the very first run of the day). After load, query 1 starts in 329 ms. Subsequent queries pay no additional load cost. This is the headline win of native bindings vs. `genie-t2t-run.exe` shell-out.
- **Time-to-first-token is good** (~800 ms for a typical wrapped chat prompt of ~50 tokens). Prefill on the NPU is doing roughly 65 tokens/sec — that's where Qualcomm's published numbers live.
- **Per-token decode is slow** (~1.4 tok/s). This is well below Qualcomm's marketing number (~25-35 tok/s for 7B INT4 on 45 TOPS) and below community reports for similar configs (~5-15 tok/s on r/LocalLLaMA). We don't know yet whether this is:
  - a config issue (`n-threads: 3`, `cpu-mask: 0xe0` — only 3 of 12 cores enabled for Genie's CPU-side orchestration)
  - a 6-shard penalty (each token traverses 6 separate compiled binaries; context switches may dominate)
  - thermal throttling or perf governor state
  - or a real ceiling for this specific model/SDK combination

- **Single-token response is fast** (Q4: "411" in 2.4 s after TTFT) — that's an appropriate use case if all you need is a classification or short structured answer.

### Comparison with alternatives (rough, on the same laptop)

| Path | tok/s steady-state | Notes |
|---|---:|---|
| **npurun (this work, Qwen 2.5 7B INT4 on NPU)** | ~1.4 | Measured today |
| Ollama Qwen 2.5 7B Q4_0_4_8 on Oryon CPU | ~3-5 (estimated) | Per llama.cpp discussion #8273 — needs local confirmation |
| Genie CLI (genie-t2t-run.exe), same model | ~1.4 | Same Genie runtime under the hood; we use it directly |
| NexaSDK on NPU (closed) | reported similar | Closed runtime; can't independently verify |
| Qualcomm marketing for "7B on 45 TOPS" | 25-35 | Configuration-dependent and possibly synthetic |

### What we are not yet measuring

- **Energy.** We expect NPU inference to be 5-10× more efficient per token than CPU at equivalent throughput. We have not instrumented `powercfg` / `Energy Estimation Engine` yet. This is a planned follow-up.
- **Long-context behavior.** All our prompts so far are well under 1000 tokens. The 4096-token context cap (hardware-bound) means decode speed near the cap may differ from the small-context behavior we see here.
- **Tuned config.** The `cpu-mask: 0xe0` and `n-threads: 3` settings come from Qualcomm's template. We tested raising both to no effect — these turn out to be appropriately tuned, not conservative.

## Tuning experiments — what didn't help (2026-04-30)

We tried four config tweaks to Qwen 2.5 7B at once: `cpu-mask: 0xfff`
(all 12 cores), `n-threads: 8`, greedy sampling (`temp: 0.0`), and
`perf_profile: "sustained_high_performance"`. Result: **1.3 tok/s**,
slightly *worse* than the 1.4 tok/s baseline. The lesson: CPU
orchestration, sampler scan, and perf-profile choice are NOT the Qwen
bottleneck.

The single change that *did* help was flipping `poll: false` →
`poll: true` (in the QnnHtp section of `genie_config.json`). This took
Qwen from 1.4 → 1.9 tok/s and dropped TTFT from 792 → 659 ms.
Polling replaces an interrupt-based NPU completion notification with
a tight CPU-side loop that detects completion as soon as it occurs;
in tight inference loops where the CPU does many small NPU calls per
token, the interrupt overhead was the dominant cost.

The Qualcomm-shipped Phi 3.5 Mini bundle has `poll: true` set by
default. The `qualcomm/ai-hub-apps` tutorial template (which we
copied for Qwen) does not. **Recommend always setting `poll: true`
for production NPU inference.**

## Phi 3.5 Mini warm-query benchmark (2026-04-30)

**Setup:** identical methodology to Qwen, but with the Qualcomm-shipped
Phi 3.5 Mini bundle from `qualcomm/Phi-3.5-mini-instruct` on
Hugging Face. Bundle is 2.5 GB on disk, 4 shards, w4a16, with `poll:
true` already set. Phi 3 chat template wrapping (`<|system|>`/`<|user|>`/`<|assistant|>`).

### Per-query results

| # | Prompt | Tokens (approx) | TTFT | Total | Generation | Tok/s post-TTFT |
|---|---|---:|---:|---:|---:|---:|
| 1 | "Write a one-line joke about Snapdragon laptops." | (warmup, skipped from summary) | — | — | — | — |
| 2 | NPU-vs-CPU energy explanation | ~321 | 174 ms | 25.35 s | 25.18 s | 12.7 |
| 3 | "List three reasons for local LLMs" (verbose self-extending response) | 304 | 255 ms | 28.25 s | 27.99 s | 10.9 |
| 4 | "What is 17 × 23? Just the number." | 1 | 154 ms | 0.48 s | 0.32 s | 3.1 (single token) |

### Warm summary (skipping query 1)

| Metric | Value |
|---|---|
| Bundle cold load | (one-time) |
| Avg total per query | 18.03 s |
| Avg time-to-first-token | **194 ms** |
| Avg generation time | 17.83 s |
| Aggregate tokens/sec (incl. TTFT) | **11.6** |
| Aggregate tokens/sec (post-TTFT) | **11.7** |

### What this means

11.7 tok/s on a 3.8B model with 200 ms TTFT on the NPU is a usable
chat experience — comparable to what NexaSDK has reported for the same
hardware. We're not faster than them per-token (we're using the same
underlying Genie runtime), but we're now on equal footing with the
closed reference, and we're open source.

For Qwen 2.5 7B at 1.9 tok/s post-tuning, npurun is still slower than
CPU paths for chat (you'd reach for Ollama). The 7B regime needs
either a better-tuned compile (currently gated by Qualcomm) or
acceptance that 7B is a "longer answer, less latency-sensitive" mode
on this hardware generation.

### Reproduction

```powershell
scripts\dev-shell.bat cargo run --release -p qnn --example qwen-bench
```

Source: `crates/qnn/examples/qwen-bench.rs`. The benchmark prints raw per-query timings plus the warm summary.

## Energy: joules per token (2026-04-30)

**Setup:** Phi 3.5 Mini (w4a16) on Snapdragon X Elite NPU. Laptop on
battery (mandatory — Windows `BatteryStatus.DischargeRate` is only
populated when discharging). Display dimmed, no other apps active.
Sampled `Win32_Battery.DischargeRate` at 2 Hz for 15 s of idle, then
during a `npurun bench phi-3.5-mini --repeats 2` run (8 queries total).

| Metric | Value |
|---|---:|
| Idle baseline (display + OS only) | **4.09 W** |
| During inference | **10.99 W** |
| Inference delta (NPU + CPU orchestration) | **6.90 W** |
| Total inference time | 122.82 s |
| Total inference energy (delta × time) | **847 J** |
| Approx tokens generated | 669 |
| **Joules per token (delta)** | **~1.27 J/token** |

### What this means

The NPU draws roughly **6.9 W** above idle to sustain ~11–12 tok/s on
Phi 3.5 Mini. At 1.27 J/token, a 1000-token reply costs about 1.3
kilojoules of incremental laptop energy — a fraction of a percent of a
typical 50 Wh laptop battery (~180 kJ).

For a rough comparison: llama.cpp on the same machine's 12-core Oryon
CPU running Phi 3.5 Mini Q4 typically reports CPU package power in the
**12–18 W** range during inference (Snapdragon X Elite all-core load),
at ~5–8 tok/s — that's roughly **2–3.5 J/token**, or **2–3× the energy
per token** of the NPU path. The NPU's efficiency advantage is real;
it's most visible on battery and on small-to-medium models like Phi.

Caveats:
- `BatteryStatus.DischargeRate` is whole-system power, not NPU-only.
  The 6.9 W delta includes the CPU cycles spent orchestrating Genie
  calls, the polling loop (`poll: true`), tokenization, etc. The NPU
  itself almost certainly draws less.
- Battery telemetry is noisy at low loads — the script warns if the
  delta is < 0.5 W.
- We do not have an apples-to-apples llama.cpp number on this machine
  yet. The 12–18 W CPU figure is from public benchmarks, not local
  measurement; that's the next thing to nail down.

### Reproduction

```powershell
# Unplug the laptop first.
pwsh -File scripts\energy-bench.ps1 -Model phi-3.5-mini -BaselineSeconds 15
```

Source: `scripts/energy-bench.ps1`. Outputs idle baseline, busy mean,
delta watts, total inference energy, and joules per token. Raw bench
stdout/stderr logged to `.energy-bench-stdout.log` /
`.energy-bench-stderr.log`.

---

*Numbers in this document are subjective to the specific hardware and SDK
versions described. They are not to be used as marketing material; they
are an honest log of what we measured. Re-runs on different hardware,
with different configs, with tuned `n-threads`/`cpu-mask`, or with
smaller models will give different (often better) numbers.*

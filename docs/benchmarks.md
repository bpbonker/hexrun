# Benchmarks

Measurements on Snapdragon X Elite (X1E80100, 16 GB LPDDR5x).
Updated as new data arrives.

## Current results (w4a16 multi-graph era, 2026-05-01)

| Model | Quant | Tool | Steady-state tok/s | TTFT |
|---|---|---|---:|---:|
| **Qwen3-4B Instruct 2507** | **w4a16** | **`npurun bench`** | **~14.9** | **~120 ms** |
| Phi 3.5 Mini | w4a16 | `qnn` example | ~11.7 | ~194 ms |
| Qwen 2.5 VL-7B Instruct (text-only) | w4a16 | `npurun bench` | ~9.1 | ~156 ms |

**Qwen3-4B Instruct 2507 at ~14.9 tok/s is the current X1E NPU ceiling
under `npurun bench`.** All three w4a16 multi-graph bundles in the
4B–7B range run at chat-pace — comfortably above human reading speed.

The cumulative lift from the very first Phase 0 measurement (Qwen 2.5
7B w8a16 at 1.4 tok/s — see *Previous iterations* below) to today's
14.9 tok/s headline is roughly 10×, made up of:

- ~2× from 7B → 4B parameters
- ~1.3× from w8a16 → w4a16 (less memory bandwidth pressure)
- ~1.2× from 6 → 4 shards (fewer per-token context transitions)
- ~1.4× from `poll: true` (CPU-side polling instead of interrupt waits)
- ~1.3× from `enable-graph-switching: true` on multi-graph bundles
  (without it, decode runs on the prefill graph for a 20× slowdown —
  see [`multi-graph-fix.md`](multi-graph-fix.md))

Other config tweaks tested in early iterations (more CPU cores via
`cpu-mask: 0xfff`, more `n-threads`, greedy sampling,
`sustained_high_performance` perf profile) had no measurable benefit
and were slightly counter-productive. The bottleneck was never CPU
orchestration — it was waiting on NPU completion via interrupts, which
`poll: true` resolves.

## Looking ahead (predictions and in-flight work)

What we expect to land in subsequent measurement passes:

- **Qwen3-4B at 32K context.** All current registry bundles ship with
  the 4096-ctx tier active. `genie_config.json` exposes `cl4096` /
  `cl8192` / `cl16384` / `cl32768` tiers in the same physical context
  bins, so once the runtime can pin a higher tier per request, we
  expect ~5–10% slower decode at 32K vs 4K (per specula's published
  per-ctx scaling on similar hardware). Tracked under Wave H1 in
  [`roadmap.md`](roadmap.md). The local x64 export to *produce* the
  32K bundle currently OOMs on 16 GB RAM; the cloud-Linux build farm
  path is the workaround.
- **Llama 3.1 8B w4a16.** Has a precompiled `DEFAULT_W4A16` checkpoint;
  same multi-graph format as Qwen3-4B. We expect roughly half the
  decode rate of Qwen3-4B (so ~6–8 tok/s) just from doubled parameter
  count and memory-bandwidth pressure — still chat-pace.
- **Qwen 2.5 7B w4a16.** Once the w4a16 multi-graph variant lands, it
  should clear the legacy w8a16 row by the same ~5–6× factor we see on
  Qwen3-4B / VL-7B and replace it in the current results table.
- **Snapdragon X2 silicon (~80 TOPS, late 2026).** Per Qualcomm
  marketing and specula's preliminary numbers, X2 should roughly
  double NPU throughput and may relax the 4096-token context cap.
  npurun is silicon-agnostic at the runtime layer (libGenie abstracts
  the chip) so X2 support is an SDK-version bump and a re-bench, not
  a rewrite.

## Previous iterations

The numbers below are kept for historical context and to show the
journey from Phase 0 (slow w8a16) to the current w4a16 headline.

### Original Phase 0 measurements (Qwen 2.5 7B w8a16, 2026-04-30)

These are the early numbers from before w4a16 multi-graph bundles
were available — when the only NPU-runnable bundle for this hardware
was a self-exported w8a16 of Qwen 2.5 7B.

> **Context — what we now know:** the 1.4 tok/s baseline below was
> *not* a fundamental hardware limit. It was the combined cost of
> w8a16 instead of w4a16, 6 shards instead of 4, and `poll: false`.
> Subsequent w4a16 multi-graph bundles (Qwen3-4B etc.) clear this row
> by ~10×. The questions raised in the *What this tells us* / *What
> we are not yet measuring* subsections below have since been
> answered: the bottleneck was interrupt-based NPU completion (fixed
> by `poll: true`) and the 20× decode-graph regression on multi-graph
> bundles (fixed by `enable-graph-switching: true`); energy is
> measured at ~1.27 J/token (see *Energy* section below); the 4096
> context cap is now per-tier configurable in newer bundles. The
> bullets below reflect what we knew and didn't know **at the time**.

| Model | Quant | Shards | Config | Steady-state tok/s | TTFT |
|---|---|---:|---|---:|---:|
| Qwen 2.5 7B | w8a16 | 6 | adapted from tutorial template, `poll: false` | 1.4 | 792 ms |
| Qwen 2.5 7B | w8a16 | 6 | + tuned cpu-mask/threads/sampler/perf-profile | 1.3 | 853 ms |
| Qwen 2.5 7B | w8a16 | 6 | **+ `poll: true`** (single flag) | **1.9** | 659 ms |

### Phase 1 warm-query benchmark (2026-04-30)

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

### Tuning experiments — what didn't help (2026-04-30)

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

### Phi 3.5 Mini warm-query benchmark (2026-04-30)

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

### What this measurement told us at the time

11.7 tok/s on a 3.8B model with 200 ms TTFT on the NPU was the first
chat-usable result on this hardware — comparable to what NexaSDK
reports on the same chip. npurun isn't faster than them per-token (we
use the same underlying Genie runtime); the contribution is parity
with the closed reference in an open Rust runtime.

Subsequent w4a16 multi-graph bundles (Qwen3-4B at 14.9 tok/s, VL-7B
at 9.1 tok/s) cleared the Phi headline once `enable-graph-switching`
was understood. Phi 3.5 Mini remains in the *Current results* table
above as a reproducible baseline; the Qwen3-4B row is the headline.

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

# Benchmarks

Honest measurements. Updated as new data arrives.

## Phase 1 warm-query benchmark (2026-04-30)

**Setup:**
- Hardware: Microsoft Surface laptop, Snapdragon X Elite X1E80100, 16 GB shared LPDDR5x, 7.8 GB shared NPU memory
- Bundle: Qwen 2.5 7B Instruct, w8a16 quantized, 6 ctx-bin shards (4.6 GB), context length 4096, sequence length 128
- Runtime: hexrun's `qnn::genie::Dialog` calling Qualcomm's libGenie 1.17.0 directly via Rust FFI (no `genie-t2t-run.exe` shell-out)
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
| **hexrun (this work, Qwen 2.5 7B INT4 on NPU)** | ~1.4 | Measured today |
| Ollama Qwen 2.5 7B Q4_0_4_8 on Oryon CPU | ~3-5 (estimated) | Per llama.cpp discussion #8273 — needs local confirmation |
| Genie CLI (genie-t2t-run.exe), same model | ~1.4 | Same Genie runtime under the hood; we use it directly |
| NexaSDK on NPU (closed) | reported similar | Closed runtime; can't independently verify |
| Qualcomm marketing for "7B on 45 TOPS" | 25-35 | Configuration-dependent and possibly synthetic |

### What we are not yet measuring

- **Energy.** We expect NPU inference to be 5-10× more efficient per token than CPU at equivalent throughput. We have not instrumented `powercfg` / `Energy Estimation Engine` yet. This is a planned follow-up.
- **Long-context behavior.** All our prompts so far are well under 1000 tokens. The 4096-token context cap (hardware-bound) means decode speed near the cap may differ from the small-context behavior we see here.
- **Smaller models.** Phi 3.5 Mini (3.8B) on the same NPU should be substantially faster — typical mobile-NPU experience is that going from 7B to 3.8B is roughly 2× speedup.
- **Tuned config.** The `cpu-mask: 0xe0` and `n-threads: 3` settings come from Qualcomm's template; they may be conservative for this 12-core Oryon laptop.

### Reproduction

```powershell
scripts\dev-shell.bat cargo run --release -p qnn --example qwen-bench
```

Source: `crates/qnn/examples/qwen-bench.rs`. The benchmark prints raw per-query timings plus the warm summary.

---

*Numbers in this document are subjective to the specific hardware and SDK
versions described. They are not to be used as marketing material; they
are an honest log of what we measured. Re-runs on different hardware,
with different configs, with tuned `n-threads`/`cpu-mask`, or with
smaller models will give different (often better) numbers.*

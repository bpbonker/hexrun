# ORT-QNN vs libGenie probe findings

**Date:** 2026-05-01
**Hardware:** Snapdragon X Elite (X1E80100), Windows 11 ARM64, QAIRT 2.45.0
**Trigger:** [`docs/specula_followups.md`](specula_followups.md) Item 5 — feasibility-probe whether the +19% TG / +39% PP lift specula reported on Qwen3-4B w4a16 (X2 silicon, ORT-QNN vs libGenie) transfers to Phi 3.5 Mini on the X1E that npurun targets.

This is a **microbenchmark**, not a full LLM benchmark. Read the *Caveats* section before drawing conclusions.

## TL;DR

On a per-stage HTP-execution microbenchmark, **raw ORT-QNN summed across all 8 pipeline shards runs ~2× *slower* than libGenie's reported full-pipeline numbers on this X1E for Phi 3.5 Mini.** The +19/+39% lift specula reported on Qwen3-4B/X2 does **not** appear to transfer in this configuration.

| Metric | libGenie (npurun bench) | ORT-QNN (this microbench) | Ratio |
|---|---:|---:|---:|
| Prefill (≤128 tokens) | ~110 ms | 213–249 ms | libGenie 1.9–2.3× faster |
| Per-token decode | ~80 ms | 145–149 ms | libGenie 1.8× faster |
| Implied steady-state | ~12.7 tok/s | ~6.7–6.9 tok/s | libGenie 1.9× faster |

The probe doesn't refute specula's finding — they measured a different model on different silicon — but it does say: **don't expect a free perf lift from porting npurun's `Backend::Genie` path to ORT-QNN on X1E.**

## What we measured

The artifact: [`llmware/phi-3.5-onnx-qnn`](https://huggingface.co/llmware/phi-3.5-onnx-qnn) — same Phi 3.5 Mini base, INT4 weights, partitioned across 4 weight-shared shards just like the Qualcomm Genie bundle, but compiled for the ORT-QNN execution provider via the `EPContext` ONNX wrapper pattern (each `*_qnn_ctx.onnx` file is a 2.5 KB shim pointing at the corresponding `weight_sharing_model_*.serialized.bin`). 2.17 GB total.

The pipeline as described in the bundle's `genai_config.json` (read by `onnxruntime-genai`):

1. CPU stages: `position_processor`, `quantizer`, `dequantizer`
2. **NPU stages (the ones we measure):**
   - 4 × `ar128_cl4096_*_of_4_qnn_ctx.onnx` (prompt processors, prefill at AR=128)
   - 4 × `ar1_cl4096_*_of_4_qnn_ctx.onnx` (token generators, decode at AR=1)

Our harness (`scripts/bench-ort-qnn-microbench.py`):

- Loads each NPU shard as a separate `onnxruntime.InferenceSession`, bound to the QNN HTP NPU device via the v1.25 plugin-EP API (`SessionOptions.add_provider_for_devices`) with the same provider options the bundle declares (`htp_performance_mode=burst`, `enable_htp_shared_memory_allocator=1`, `qnn_context_priority=high`).
- Synthesises zero-filled tensors of every input's declared shape and dtype — QNN HTP has no data-dependent paths so zeros are valid for timing.
- Runs 3 warmup passes + 30 timed passes per shard. Reports median, mean, and min.
- Sums the 4 stage medians per phase (prefill, decode) for an aggregate.

## Numbers

Two independent runs, same hardware, ~30 seconds apart:

### Run 1 (warmup=2, iters=5)

| Shard | median | mean | min |
|---|---:|---:|---:|
| ar128_cl4096_1_of_4 | 54.78 ms | 56.51 ms | 49.59 ms |
| ar128_cl4096_2_of_4 | 51.30 ms | 51.23 ms | 49.12 ms |
| ar128_cl4096_3_of_4 | 49.80 ms | 50.21 ms | 47.28 ms |
| ar128_cl4096_4_of_4 | 58.04 ms | 59.27 ms | 51.26 ms |
| **prefill total** | **213.92 ms** | | |
| ar1_cl4096_1_of_4 | 32.75 ms | 34.63 ms | 29.68 ms |
| ar1_cl4096_2_of_4 | 37.33 ms | 38.80 ms | 32.78 ms |
| ar1_cl4096_3_of_4 | 39.99 ms | 39.65 ms | 32.05 ms |
| ar1_cl4096_4_of_4 | 35.16 ms | 36.79 ms | 31.13 ms |
| **decode total** | **145.24 ms** | | (≈ 6.89 tok/s) |

### Run 2 (warmup=3, iters=30)

| Shard | median | mean | min |
|---|---:|---:|---:|
| ar128_cl4096_1_of_4 | 63.12 ms | 62.76 ms | 47.83 ms |
| ar128_cl4096_2_of_4 | 61.34 ms | 61.17 ms | 48.24 ms |
| ar128_cl4096_3_of_4 | 61.70 ms | 61.14 ms | 50.44 ms |
| ar128_cl4096_4_of_4 | 63.26 ms | 61.28 ms | 50.75 ms |
| **prefill total** | **249.41 ms** | | |
| ar1_cl4096_1_of_4 | 37.41 ms | 38.88 ms | 28.77 ms |
| ar1_cl4096_2_of_4 | 37.93 ms | 38.14 ms | 29.27 ms |
| ar1_cl4096_3_of_4 | 39.02 ms | 39.20 ms | 31.94 ms |
| ar1_cl4096_4_of_4 | 34.25 ms | 36.66 ms | 30.73 ms |
| **decode total** | **148.62 ms** | | (≈ 6.73 tok/s) |

### libGenie baseline on the same hardware

From [`docs/benchmarks.md`](benchmarks.md) and the `9611c7b` post-fix smoke:

- TTFT: ~105–200 ms (the README quotes 200 ms conservatively; post-fix bench measured 105 ms)
- Steady-state post-TTFT: ~11.7–12.7 tok/s (~78–85 ms per generated token)
- Energy: ~6.9 W delta, ~1.27 J/token

## Why we measured a microbench, not a full LLM run

The original plan was to mirror specula's shape: load the bundle through `onnxruntime-genai` and run the same prompts as `npurun bench`. That hit a hard blocker:

> **`RuntimeError: QNN execution provider is not supported in this build.`**

`onnxruntime-genai`'s published wheels (stable `0.13.1` on PyPI, nightly `0.13.0.dev20260402` on the ORT-Nightly Azure feed) are not compiled with `--use_qnn`. PyPI doesn't host any `onnxruntime-genai-qnn` / `-snapdragon` / `-arm64` variant. As of May 2026, getting QNN-enabled `onnxruntime-genai` on Windows ARM64 requires building from source against the QNN SDK — a 1–3 hour task with real toolchain risk on ARM64 (Microsoft's CI workflow `win-cpu-arm64-build.yml` doesn't exercise the QNN path).

The unfinished `scripts/bench-ort-qnn.py` (the full-pipeline bench harness) is left in the repo for whoever picks the source-build path up next. Run-instructions are in its module docstring.

The microbench path picked instead: `onnxruntime-qnn` 2.1.0 — Microsoft's plugin-style QNN execution provider — *does* ship as a Win ARM64 wheel on PyPI and *does* successfully execute these context binaries on the X1E HTP. So we measure each shard's HTP execution latency in isolation, sum across shards, and compare aggregates.

## Caveats

This microbench is a **lower-bound proxy** for what a real `Backend::Ort` would deliver. Several things would change in a full pipelined runtime:

1. **Persistent sessions.** We open 8 fresh `InferenceSession`s and run them with zero inputs. A real LLM runtime keeps these sessions warm across calls, with KV cache state living on the HTP between stages. Our measurement includes per-call setup overhead that pipelined execution amortizes.
2. **Inter-stage data transfer.** A real pipeline pipes shard 1's output into shard 2's input. We don't model that — but skipping it should make us *faster*, not slower, so this isn't what's costing us.
3. **No tokenizer/sampler overhead.** We don't tokenize, don't sample. libGenie does. Our number is therefore an even more favourable comparison point for ORT-QNN, and we're still 2× slower.
4. **Different cross-section.** specula measured Qwen3-4B/w4a16/X2-silicon. We measured Phi-3.5-mini/w4a16/X1E-silicon. The same library on different model+silicon can flip the ratio.

What this microbench is *not* a lower bound for: per-call session setup, which dominates if it's high. A definitive answer requires either source-building `onnxruntime-genai` with QNN or implementing a persistent multi-session pipeline (effectively rebuilding libGenie's orchestration in our own code).

## Implications for npurun

The original [`docs/architecture.md`](architecture.md) had ORT-QNN as the *default* backend with libGenie behind a feature flag. This probe argues against that ordering on X1E:

- **Don't budget a `Backend::Ort` port expecting a free 19/39% lift on this hardware tier.** Whatever the win is on X2, it's not present here.
- **Do still consider the port for non-perf reasons:** broader ONNX model coverage, multi-session concurrency, alignment with Microsoft's tooling. But its perf would need to at minimum match libGenie, not exceed it, on Phi-class workloads.
- **Re-test on X2 before deciding.** If the lift is silicon-generation-dependent (X2's HTP v79/v81 has higher bandwidth than X1E's v73), npurun may want a runtime-selectable backend rather than a fixed default.

## Reproducing this probe

Prerequisites: ARM64 Python 3.11 (path: `py -3.11-arm64`), QAIRT 2.45 at `QNN_SDK_ROOT`, and the dev-shell env.

```powershell
# 1. Set up venv (one-time)
py -3.11-arm64 -m venv python\.venv-ort-qnn
python\.venv-ort-qnn\Scripts\python.exe -m pip install --upgrade pip
python\.venv-ort-qnn\Scripts\python.exe -m pip install --no-deps onnxruntime-qnn
python\.venv-ort-qnn\Scripts\python.exe -m pip install onnxruntime numpy huggingface-hub

# 2. Download the llmware bundle (~2.17 GB, one-time)
python\.venv-ort-qnn\Scripts\python.exe -c "from huggingface_hub import snapshot_download; import os; snapshot_download('llmware/phi-3.5-onnx-qnn', local_dir=os.path.join(os.environ['LOCALAPPDATA'], 'npurun', 'models', 'phi-3.5-mini-ort'))"

# 3. Run the microbench (must go through dev-shell so QnnHtp.dll resolves)
scripts\dev-shell.bat python\.venv-ort-qnn\Scripts\python.exe scripts\bench-ort-qnn-microbench.py --warmup 3 --iters 30 --csv .\ort-qnn-microbench.csv

# 4. Compare against the libGenie baseline (different harness, same hardware)
scripts\dev-shell.bat target\release\npurun.exe bench phi-3.5-mini --csv .\genie-baseline.csv
```

## What we'd do next

If we want to definitively rule the lift in or out for npurun's hardware tier, in increasing order of effort:

1. **Re-run on X2 silicon when available.** Same scripts, same model artifacts. If the X2 numbers flip, the lift is silicon-generation-specific and X1E isn't where it lands.
2. **Source-build `onnxruntime-genai` with `--use_qnn`** and rerun the original `scripts/bench-ort-qnn.py` end-to-end (full LLM, not microbench). 1–3 hours of build wrangling. Numbers would be directly comparable to specula's.
3. **Build a persistent multi-session pipeline harness** that mirrors what libGenie does internally — KV cache lives across stages, sessions stay warm, tokenizer + sampler are wired up. ~half-day of focused work. Closest to "what `Backend::Ort` would actually be" but no longer a microbench.

For the immediate roadmap, the right call is **leave Item 5 open and de-prioritise the perf-driven motivation** unless someone hits us with X2 hardware where it's worth re-running.

## Artifacts

- `scripts/bench-ort-qnn-microbench.py` — the working harness used here.
- `scripts/bench-ort-qnn.py` — the full-pipeline harness, blocked on source-built `onnxruntime-genai`. Kept for whoever picks that up next.
- `%LOCALAPPDATA%\npurun\models\phi-3.5-mini-ort\` — the llmware bundle on disk.
- `python/.venv-ort-qnn/` — ARM64 venv with `onnxruntime`, `onnxruntime-qnn`, `onnxruntime-genai`. Gitignored.

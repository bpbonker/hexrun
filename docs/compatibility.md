# Model compatibility matrix

> npurun targets Snapdragon X-series Windows-on-ARM laptops broadly
> (X Elite, X Plus, and forward — anything with a Hexagon NPU and QAIRT
> support). The numbers in this table were measured on X Elite (X1E80100)
> specifically because that is what we have in hand. Other X SKUs and
> the X2 generation should work but are unverified.

Tracks which models npurun has run end-to-end on the Hexagon NPU.
Verified rows are reproduced via `npurun pull <name>` + `npurun bench
<name>` on X1E80100 hardware. Filed via the
[Model compatibility report](https://github.com/bpbonker/npurun/blob/main/.github/ISSUE_TEMPLATE/model_compatibility.md)
issue template; promoted here once verified.

## Verified working

| Model | Arch | Quant | NPU tok/s | TTFT | Bundle source | Notes |
|---|---|---|---:|---:|---|---|
| **Qwen3-4B Instruct 2507** | qwen3 | w4a16 | **~14.9** | ~120 ms | Qualcomm AI Hub precompiled, multi-graph | Current X1E ceiling under `npurun bench`. `enable-graph-switching: true` injected automatically by `npurun pull`. |
| **Qwen3-4B (base)** | qwen3 | w4a16 | (same family) | — | Qualcomm AI Hub precompiled, multi-graph | Base model; same multi-graph format as Instruct-2507. |
| **Phi-3.5-mini-instruct** | phi3 | w4a16 | **~11.7** | ~194 ms | Qualcomm AI Hub precompiled | 4-shard, `poll: true` in stock config. The original chat-pace headline. |
| **Qwen 2.5 VL-7B Instruct** | qwen2_vl | w4a16 | **~9.1** (text-only) | ~156 ms | Qualcomm AI Hub precompiled, multi-graph | Vision pipeline ships in the bundle but is not exercised by npurun yet (text-generator node only). |

## Legacy / kept for historical context

| Model | Arch | Quant | NPU tok/s | Status |
|---|---|---|---:|---|
| Qwen 2.5 7B Instruct | qwen2 | w8a16 | ~1.9 (`poll: true`) | Phase 0 self-export. Predates w4a16 multi-graph era. Kept in registry as `qwen-2-5-7b` for archeology; new users should pick a w4a16 model instead. |

## In progress / planned

| Model | Status | Wave | Notes |
|---|---|---|---|
| Qwen3-4B Instruct 2507 @ 32K context | 🚧 in flight | H1 | Asset is downloadable, but the local x64 ONNX-consolidation step OOMs on 16 GB RAM. Cloud Linux x86 build farm is the workaround (Wave I). |
| Llama 3.1 8B Instruct (w4a16) | 🚧 queued | H2 | `DEFAULT_W4A16` precompiled checkpoint exists; same multi-graph format. Expected ~6–8 tok/s based on 4B → 8B parameter scaling. |
| Qwen 2.5 7B (w4a16, multi-graph) | ⏳ planned | H3 | Once a w4a16 multi-graph variant is available, replaces the legacy w8a16 row above. Expected to clear that row by ~5–6×. |
| Llama 3.2 3B Instruct | ⏳ planned | G2 | No Qualcomm-shipped Genie bundle today; needs `npu-convert export` against AI Hub. |
| Llama 3.2 1B Instruct | ⏳ planned | G2 | HF-gated (Meta access required). Smallest first-party-friendly target. |
| Gemma 2 2B / Mistral 7B v0.3 | ⏳ planned | G2 | No Qualcomm-shipped bundles; `npu-convert export` path. |

## Status legend

- ✅ Verified working on NPU with no CPU fallback for hot ops (rows in *Verified working*)
- ⚠️ Working but with notable CPU fallback (>20% of compute)
- ❌ Conversion succeeds but inference is broken (output garbage / hang)
- 🚧 In progress
- ⏳ Planned

## Reproducibility

Every row should link to the issue or PR that produced its numbers, and
include the laptop model + Windows build + QNN SDK version used. NPU
performance is sensitive enough to driver versions that "Llama-3.2-3B works
fine" is not a useful claim without that context.

The numbers above are on X1E80100, Windows 11 24H2, QAIRT 2.45.0,
HTP driver 30.0.219.1000 (9/11/2025), libGenie 1.17.0.

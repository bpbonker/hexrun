# Model compatibility matrix

Tracks which HuggingFace LLMs we've successfully converted with `npu-convert`
and how they behave on the Hexagon NPU. Filed via the
[Model compatibility report](../.github/ISSUE_TEMPLATE/model_compatibility.md)
issue template; promoted here once verified.

| Model | Arch | Quant | Status | NPU tok/s | CPU tok/s | Ops on CPU fallback | Notes |
|---|---|---|---|---|---|---|---|
| _Phi-3.5-mini-instruct_ | phi3 | int8-w-int16-a | 🚧 Phase 0 target | — | — | — | Reference model the engine is built around |
| _Llama-3.2-3B-Instruct_ | llama | int8-w-int16-a | ⏳ Phase 5 | — | — | — | Sliding-window attention may force CPU fallback on some ops |
| _Qwen2.5-3B-Instruct_ | qwen2 | int8-w-int16-a | ⏳ Phase 5 | — | — | — | GQA support via `genai_builder` decomposition |

## Status legend

- ✅ Verified working on NPU with no CPU fallback for hot ops
- ⚠️ Working but with notable CPU fallback (>20% of compute)
- ❌ Conversion succeeds but inference is broken (output garbage / hang)
- 🚧 In progress
- ⏳ Planned

## Reproducibility

Every row should link to the issue or PR that produced its numbers, and
include the laptop model + Windows build + QNN SDK version used. NPU
performance is sensitive enough to driver versions that "Llama-3.2-3B works
fine" is not a useful claim without that context.

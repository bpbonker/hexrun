---
name: Model compatibility report
about: Tell us how a specific HF model behaves on the NPU
title: "[model] "
labels: compatibility
---

## Model

- Hugging Face ID:
- Architecture:
- Parameters:
- Quantization tried (int8 / int8-w-int16-a / int4):

## Conversion via npu-convert

- Did `npu-convert convert <id>` succeed?
- Stage at which it failed (export / genai_builder / quantize / context-binary compile):
- Error output:

## Inference

- Tokens/sec on NPU:
- Tokens/sec on CPU baseline (`--backend cpu`):
- NPU utilization in Task Manager (sustained %):
- Ops that fell back to CPU per `QnnHtp.log`:

## Notes

<!-- Anything unusual: long load time, memory pressure, output quality issues, etc. -->

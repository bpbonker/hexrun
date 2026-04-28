"""HF -> ONNX -> genai_builder -> quantize -> QNN context binary -> manifest.

Phase 5 lands the real implementation. Steps (per the plan):

1. ``optimum-cli export onnx`` — HF model to ONNX.
2. ``python -m onnxruntime_genai.models.builder`` — emit LLM-ready ONNX with
   KV cache exposed as graph I/O. Load-bearing; do not reinvent.
3. ``onnxruntime.quantization.quantize`` with INT8 weights (per-channel) and
   INT16 activations for attention stability. Calibrate over N samples from
   the chosen dataset.
4. Compile to a QNN context binary via ``qai-appbuilder`` (or a small C++
   helper that calls ``QnnContext_createFromBinary``).
5. Emit ``hexrun.json`` manifest with sha256s.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class ConvertOptions:
    hf_id: str
    output: str
    quant: str
    calibration: str
    seq_len: int
    samples: int


def convert(opts: ConvertOptions) -> None:
    raise NotImplementedError(
        "hex-convert pipeline is Phase 5. "
        "For Phase 0, pull a pre-converted model from Qualcomm AI Hub instead."
    )

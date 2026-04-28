"""hex-convert: HuggingFace -> ONNX -> QNN-quantized -> hexrun manifest pipeline.

This package runs on **x64 Python** (3.11+). On Windows ARM64, the ONNX
quantization tooling is broken, so emulation via Prism is the supported path
until upstream tooling lands ARM64 wheels. See README for setup.
"""

__version__ = "0.1.0.dev0"

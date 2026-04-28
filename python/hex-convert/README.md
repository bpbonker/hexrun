# hex-convert

Conversion pipeline turning HuggingFace LLMs into NPU-ready ONNX + QNN context
binaries with a `hexrun.json` manifest. Sidecar to the `hexrun` Rust runtime.

## Why x64 Python on an ARM64 laptop

ONNX Runtime's quantization tooling does not currently install on Windows
ARM64. Until upstream wheels land, the supported path is **x64 Python under
Prism emulation**:

```powershell
# Install x64 Python (e.g. Python 3.11 x64) somewhere distinct from your ARM64 Python.
py -3.11-64 -m venv .venv-x64
.\.venv-x64\Scripts\Activate.ps1
pip install -e .
```

## Status

Phase 5. The CLI surface and pipeline outline exist; the body is a
`NotImplementedError`. For Phase 0 verification, pull a pre-converted
Qualcomm AI Hub model instead.

# Troubleshooting

A growing list of failure modes and how to diagnose them. PRs welcome.

## "It runs but the NPU column in Task Manager stays at 0%"

You're hitting silent CPU fallback. Three things to check, in order:

1. **Is the QNN EP actually registered?** Run with `RUST_LOG=ort=debug`. If
   you see `failed to register QNN execution provider`, the EP isn't loaded —
   re-check `QNN_SDK_ROOT` and that `onnxruntime-qnn` (or the linked ORT) is
   the QNN-enabled build.
2. **Is the model fully delegated?** Set `QNN_LOG_LEVEL=PROFILE` and run a
   short generation. Open the resulting `QnnHtp.log`. Each op should report
   running on the `HTP` backend. Anything on `CPU` is a fallback. Common
   culprits: dynamic shapes, RoPE variants the EP doesn't recognize, novel
   GQA layouts.
3. **Is the model actually quantized?** The HTP backend rejects FP32 outright.
   `hex-convert` should refuse to emit a non-quantized model, but if you
   loaded an external ONNX, double-check.

## "SDKError(-100301) Plugin loading failed" / version mismatch

This is the Nexa #1060 trap. The QNN SDK, HTP driver, and any pre-built
plugin libs need compatible versions. `hexrun` checks the manifest's
`qnn_sdk` field against the live runtime; if you see this error before the
check fires, you're likely on a manually mixed install. Run `setup-qnn.ps1`
and confirm the reported SDK version matches what the model was compiled
against.

## "cargo build fails on qnn-sys with QNN_SDK_ROOT errors"

Two paths:

- You don't have the SDK and just want to try the rest of hexrun:
  `cargo build` with the default workspace members. `qnn-sys` and `qnn`
  are excluded from `default-members`, so the build skips them.
- You have the SDK but the build still fails: check that the SDK's
  `include\QNN` directory is present and that the lib subdirectory matches
  the architecture (`lib\aarch64-windows-msvc` on Snapdragon).

## "Conversion fails on Windows ARM64 Python"

ARM64 Python wheels for ONNX quantization tooling are still missing. Use
**x64 Python under Prism emulation**:

```powershell
winget install Python.Python.3.11 --architecture x64
py -3.11-64 -m venv .venv-x64
.\.venv-x64\Scripts\Activate.ps1
pip install -e python\hex-convert
```

## "First load is very slow"

Compiling an ONNX into a QNN context binary takes 30–90 seconds. Subsequent
loads use the cached `.qnn_ctx.bin`. If every load is slow, the cache file
is being invalidated — check that the model directory is writable and that
the `qnn_sdk` version embedded in the cache matches the live runtime.

# Troubleshooting

A growing list of failure modes and how to diagnose them. PRs welcome.

## Before filing a bug — run `npurun show-hardware`

Most NPU loading issues come down to one of three things: the SDK isn't
where npurun expects it, the Hexagon arch you have isn't in the SDK's
shipped set, or libGenie didn't load at all. `npurun show-hardware`
probes all three in one shot:

```powershell
npurun show-hardware
```

Paste the full output into the issue. It captures the SoC marketing
name, the Qualcomm Hexagon NPU PnP entry Windows reports, the Hexagon
architectures the installed QAIRT SDK ships support for, and both the
QAIRT and libGenie versions. That's enough for someone else to tell
whether you're on a configuration npurun has been tested against.

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
   `npu-convert` should refuse to emit a non-quantized model, but if you
   loaded an external ONNX, double-check.

## "SDKError(-100301) Plugin loading failed" / version mismatch

This is the Nexa #1060 trap. The QNN SDK, HTP driver, and any pre-built
plugin libs need compatible versions. `npurun` checks the manifest's
`qnn_sdk` field against the live runtime; if you see this error before the
check fires, you're likely on a manually mixed install. Run `setup-qnn.ps1`
and confirm the reported SDK version matches what the model was compiled
against.

## "cargo build fails on qnn-sys with QNN_SDK_ROOT errors"

Two paths:

- You don't have the SDK and just want to try the rest of npurun:
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
pip install -e python\npu-convert
```

## "link: extra operand ..." or "link.exe failed: exit code: 1" with weird args

You're running `cargo` from a shell where the wrong `link.exe` is on `PATH`.
On Windows, MSYS2 / Git Bash ship a GNU coreutils `link` at `/usr/bin/link.exe`
that shadows MSVC's linker. Symptom: errors like

```
link: extra operand 'C:\\...\\rcgu.o'
Try 'link --help' for more information.
```

The fix: build from a shell where MSVC's tools are first on `PATH`. The
project ships `scripts\dev-shell.bat` for exactly this:

```bat
scripts\dev-shell.bat cargo check --workspace
scripts\dev-shell.bat cargo build --release
scripts\dev-shell.bat cargo test --workspace
```

Internally it loads `vcvarsall.bat arm64` and prepends `%USERPROFILE%\.cargo\bin`.
You can also run cargo from a "Developer PowerShell for VS 2022" or the
"x64_arm64 Cross Tools Command Prompt" — same effect.

## "ring v0.17.x failed to find tool 'clang'"

`ring` (pulled in transitively via `reqwest` → `rustls`) compiles ARM64
assembly with clang. Install LLVM:

```powershell
winget install LLVM.LLVM
```

You will need it anyway for `bindgen` (the `qnn-sys` crate) once you set
`QNN_SDK_ROOT`.

## `qai-hub-models` export crashes with `UnicodeEncodeError 'charmap' codec`

The qai-hub client prints a progress animation that includes Unicode emoji
(e.g. ⏳). On Windows the default console encoding is `cp1252` which can't
encode these. The export job *itself* may have already submitted on the
cloud — the crash is purely the local progress printer.

Fix: force Python to use UTF-8 for I/O before running:

```bat
set PYTHONIOENCODING=utf-8
set PYTHONUTF8=1
python -X utf8 -m qai_hub_models.models.<model>.export ...
```

Or in PowerShell:

```powershell
$env:PYTHONIOENCODING = "utf-8"
$env:PYTHONUTF8 = "1"
python -X utf8 -m qai_hub_models.models.<model>.export ...
```

If the crash already happened, re-run with `--model-cache-mode enable` so
already-uploaded model shards aren't re-uploaded (uploads are the slow
part — multiple GB).

## "First load is very slow"

Compiling an ONNX into a QNN context binary takes 30–90 seconds. Subsequent
loads use the cached `.qnn_ctx.bin`. If every load is slow, the cache file
is being invalidated — check that the model directory is writable and that
the `qnn_sdk` version embedded in the cache matches the live runtime.

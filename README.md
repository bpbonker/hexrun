# hexrun

**NPU-first local LLM runtime for Snapdragon X Elite (Windows on ARM).**

Today, popular open-source LLM tools (Ollama, llama.cpp, LM Studio, text-generation-webui)
run CPU-only on Snapdragon X Elite laptops — the 45 TOPS Hexagon NPU sits idle.
hexrun fixes that. It's an Ollama-class CLI plus an OpenAI-compatible HTTP
server, but it actually drives the NPU via ONNX Runtime's QNN Execution
Provider (and, optionally, direct QNN context binaries).

> **Status:** pre-alpha, in active early development. The workspace compiles;
> Phase 0 (hardware/SDK proof) is in progress. See [the plan](#roadmap) for
> what's done and what's coming.

---

## Why this exists

| Tool | NPU support on Snapdragon X Elite (Apr 2026) |
|---|---|
| Ollama | ❌ CPU only (issue [#5360](https://github.com/ollama/ollama/issues/5360)) |
| llama.cpp | ❌ QNN backend stalled ([#8273](https://github.com/ggml-org/llama.cpp/discussions/8273), [#8336](https://github.com/ggml-org/llama.cpp/discussions/8336)) |
| LM Studio | ❌ CPU/GPU only ([#30](https://github.com/lmstudio-ai/lms/issues/30)) |
| text-generation-webui | ❌ ([#6298](https://github.com/oobabooga/text-generation-webui/issues/6298)) |
| ONNX Runtime QNN EP | ✅ works, but raw — no LLM scaffolding, no model registry, no daemon |
| NexaSDK | ✅ works, but closed CLI, no Rust ecosystem |
| Microsoft Phi Silica | ✅ works, but locked to first-party Copilot apps |

There's no clean OSS path for "I have a Surface Pro 11 / Yoga Slim 7x — give
me an `ollama run`-style experience that actually uses the NPU." hexrun is
that path.

## Architecture

```
hexrun/
├── crates/
│   ├── qnn-sys/         # raw FFI to QNN, bindgen-generated at build time
│   ├── qnn/             # safe RAII wrappers (Backend, Context, Graph, Tensor)
│   ├── hexrun-core/     # inference loop, tokenizer, sampler, KV cache
│   ├── hexrun-registry/ # model pull/list/cache (sha256-verified)
│   ├── hexrun-server/   # axum HTTP server (OpenAI + Ollama compat, SSE)
│   └── hexrun-cli/      # `hexrun` binary
└── python/
    └── hex-convert/     # x64 Python: HF → ONNX → QNN-quantized → manifest
```

Two parallel inference paths picked at model-load time:

- **`ort` path (default):** load `model.onnx` via the [`ort`](https://github.com/pykeio/ort) crate
  with the QNN Execution Provider. ORT QNN EP handles op dispatch, fallback,
  and HTP context-binary caching.
- **`qnn` direct path** (feature `qnn-direct`): load a pre-built `.qnn_ctx.bin`
  via our `qnn` crate. Faster cold start; needed for custom-graph experiments.

## Prerequisites

| Requirement | Why |
|---|---|
| Snapdragon X Elite or X Plus laptop | Hexagon NPU is the whole point |
| Windows 11 24H2+ on ARM64 | Required for current QNN driver and Windows ML 1.8+ |
| [Rust toolchain](https://rustup.rs) (stable, target `aarch64-pc-windows-msvc`) | builds the runtime |
| Visual Studio 2022 Build Tools (ARM64 + ARM64EC C++ workloads) | bindgen / linker |
| LLVM/Clang (for bindgen) | `winget install LLVM.LLVM` |
| Python 3.11 **x64** (under Prism emulation) | conversion pipeline; ARM64 quant tooling is broken |
| QNN SDK 2.44+ | NPU runtime |

The QNN SDK is **not redistributable** — install it manually from the
[Qualcomm developer portal](https://www.qualcomm.com/developer) (Qualcomm AI
Engine Direct), then point `QNN_SDK_ROOT` at the install dir.

## Phase 0 walkthrough — prove the NPU works on your laptop

This phase verifies the toolchain end-to-end before any hexrun code runs.
Pinned to the plan — DoD: NPU column in Task Manager shows >0% sustained
during inference, and tokens/sec are logged.

```powershell
# 1. Install Rust + ARM64 target
winget install Rustlang.Rustup
rustup target add aarch64-pc-windows-msvc

# 2. Install LLVM (for bindgen)
winget install LLVM.LLVM

# 3. Install x64 Python 3.11 (separate from any ARM64 Python you may have)
winget install Python.Python.3.11 --architecture x64

# 4. Install the QNN SDK from https://www.qualcomm.com/developer
#    (Qualcomm AI Engine Direct, version 2.44.0 or newer).
#    Then persist QNN_SDK_ROOT:
setx QNN_SDK_ROOT "C:\Qualcomm\AIStack\QAIRT\2.44.0"

# 5. Validate the install
pwsh -File .\scripts\setup-qnn.ps1

# 6. Build the workspace (qnn-sys excluded from default-members until SDK is set)
cargo build --release

# 7. Phase 0 smoke test (Python ORT QNN EP):
#    Pull a pre-converted Qualcomm AI Hub model (e.g. Llama 3.2 3B Chat)
#    and run a forward pass. Watch NPU column in Task Manager.
.\.venv-x64\Scripts\Activate.ps1
pip install onnxruntime-qnn onnxruntime-genai
python scripts\smoke_phase0.py    # comes in Phase 0 work
```

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| 0 | Hardware/SDK proof — install QNN SDK, run a Qualcomm AI Hub model via Python ORT, confirm NPU utilization | 🚧 in progress |
| 1 | `qnn-sys` bindgen + `qnn` safe wrapper | ⏳ |
| 2 | `hexrun-core` inference loop with Phi-3.5-mini hardcoded | ⏳ |
| 3 | CLI: `pull` / `run` / `list` / `rm` / `show` | ⏳ |
| 4 | HTTP server: OpenAI + Ollama compat, SSE streaming | ⏳ |
| 5 | `hex-convert` Python pipeline + Llama 3.2 3B / Qwen 2.5 3B in registry | ⏳ |
| 6 | Release prep: MSIX installer, `winget` manifest, signed CI matrix | ⏳ |

## Verifying NPU usage (not silent CPU fallback)

Three independent checks. All three should agree before claiming a model is
"running on the NPU":

1. **Task Manager** → Performance → NPU column shows sustained utilization
   during `hexrun run`.
2. `QNN_LOG_LEVEL=PROFILE` in the env; `QnnHtp.log` should show ops executing
   on the `HTP` backend, not `CPU`. `hexrun show --profile` surfaces this.
3. `--backend cpu` vs. NPU run: tokens/sec should differ by >3×. If not, the
   NPU isn't engaged.

## License

Dual-licensed under MIT or Apache-2.0 at your option.

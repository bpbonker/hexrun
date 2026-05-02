# npurun — handoff context

**Last updated:** 2026-04-30, after Wave A–C drops.
**Status:** Phases 0–4 plus pull integrity, LAN safety, and server backpressure all shipped. Working tool with `pull → run → serve` user flow, OpenAI/Ollama-compatible HTTP, sha256-verified resumable downloads, bearer auth, CORS, graceful shutdown, and HTTP 429 backpressure. See `docs/roadmap.md` for what remains for v0.1.0.

This document is the single source of truth for "where are we, what works,
what's next" if you're picking up the project mid-stream. Pair it with:

- [`README.md`](https://github.com/bpbonker/npurun#readme) — user-facing intro and quickstart
- [`roadmap.md`](roadmap.md) — wave-by-wave plan to v0.1.0
- [`architecture.md`](architecture.md) — architectural decisions
- [`troubleshooting.md`](troubleshooting.md) — every failure mode we've hit and how to fix
- [`benchmarks.md`](benchmarks.md) — measured numbers
- [`findings.md`](findings.md) — engineering writeup
- [`paper.md`](paper.md) — formal experience-report
- [`compatibility.md`](compatibility.md) — model compatibility matrix scaffold
- [`CHANGELOG.md`](https://github.com/bpbonker/npurun/blob/main/CHANGELOG.md) — chronological log of changes

---

## What npurun is

An NPU-first local LLM runtime for Snapdragon X-series Windows-on-ARM
laptops (verified on X Elite, intended to run on X Plus and X2 too).
Today, Ollama / llama.cpp / LM Studio all run CPU-only on these laptops;
the Hexagon NPU (45 TOPS) sits idle. npurun fixes that with native Rust
bindings to libGenie/QNN, an Ollama-class CLI
(`pull`/`list`/`show`/`run`/`bench`/`rm`/`serve`), and an OpenAI- and
Ollama-compatible HTTP server.

---

## Verified status

End-to-end on this laptop:

- **Qwen3-4B Instruct 2507** (w4a16, multi-graph): **~14.9 tok/s** steady-state
  under `npurun bench`, ~120 ms TTFT. Current NPU ceiling on X1E.
- **Phi 3.5 Mini** (w4a16, Qualcomm-shipped bundle): ~11.7 tok/s steady-state
  post-TTFT, ~194 ms TTFT. Chat-usable; the original headline.
- **Qwen 2.5 VL-7B Instruct** (w4a16, multi-graph): ~9.1 tok/s text-only,
  ~156 ms TTFT. Vision pipeline present in the bundle but not exercised yet.
- **Qwen 2.5 7B** (w8a16, our original Phase 0 export): ~1.9 tok/s with
  `poll: true`. The legacy slow path — predates the multi-graph w4a16
  bundles, kept around for comparison and historical context.
- HTTP server (OpenAI + Ollama compat) verified end-to-end with SSE
  streaming, NDJSON streaming, bearer auth (200/401/200 ladder), CORS
  preflight, HTTP 429 on concurrent requests, rich `/healthz`.
- `npurun pull <model>` downloads, sha256-verifies, extracts,
  auto-writes manifest, and injects `enable-graph-switching` for
  multi-graph bundles. Resumable via HTTP `Range`.

All three NPU-usage proofs agreed:

1. Native Rust → libGenie produced coherent output via `qnn::Dialog::query_streaming`
2. The bundle was compiled with `target_runtime: qnn_dlc` against
   `Snapdragon X Elite CRD` on real Qualcomm hardware (cloud compile)
3. Task Manager → Performance → NPU showed sustained ~19% utilization with
   4.9 GB shared memory in use during a long generation, while CPU stayed at 12%

---

## Hardware + toolchain installed

| Component | Version | Path / how to invoke |
|---|---|---|
| Hardware | Snapdragon X Elite **X1E80100** | confirmed via `Get-PnpDevice` |
| OS | Windows 11 ARM64 | |
| HTP driver | 30.0.219.1000 (9/11/2025) | system, managed by Windows Update |
| QAIRT SDK | **2.45.0** | `C:\AAA\Personal\AI\qairt\2.45.0` (env: `QNN_SDK_ROOT`) |
| Genie runtime | libGenie.so 1.17.0 | bundled in QAIRT under `bin/aarch64-windows-msvc/` |
| Rust | stable-aarch64-pc-windows-msvc 1.95.0 | rustup; `%USERPROFILE%\.cargo\bin` |
| MSVC | v143 14.44.35207 (ARM64/ARM64EC + ARM build tools) | VS 2022 Community at `C:\Program Files\Microsoft Visual Studio\2022\Community` |
| Win11 SDK | 10.0.26100.0 | standard MS install path |
| LLVM/clang | (via `winget install LLVM.LLVM`) | `C:\Program Files\LLVM\bin` (needed for `ring` ARM64 asm + future `bindgen` against QNN headers) |
| Python (ARM64) | 3.13.6 | system-installed; not used by qai-hub-models |
| Python (x64) | **3.11.9** | `C:\Users\Brenden\AppData\Local\Programs\Python\Python311\` (use `py -3.11-64`) |
| qai-hub-models | 0.52.0 | inside `python/.venv-x64` |
| qai-hub | 0.49.0 | inside `python/.venv-x64`, configured with API token |
| onnx-runtime | 1.22.1 | inside `python/.venv-x64` |
| torch | 2.11.0+cpu | inside `python/.venv-x64` |
| transformers | 4.45.0 | inside `python/.venv-x64` |

### Why x64 Python under emulation

The `qai-hub-models[*]` extras pin `torch==2.4.1` for the Llama-family models
(Qwen has a looser pin and resolved to 2.11.0). PyTorch 2.4.x has **no**
ARM64 Windows wheel. We installed x64 Python via winget and run it under
Prism emulation. ONNX quantization tooling has the same constraint.

The native ARM64 Python remains for any pure-Python work that doesn't need
those packages.

---

## Repository layout

```
C:\AAA\Personal\AI\npurun\
├── Cargo.toml                       # workspace manifest
├── rust-toolchain.toml              # pins stable + ARM64 target
├── rustfmt.toml, clippy.toml, deny.toml, .editorconfig
├── README.md, CONTRIBUTING.md, SECURITY.md, CODE_OF_CONDUCT.md
├── CHANGELOG.md, LICENSE-MIT, LICENSE-APACHE
├── .github/
│   ├── workflows/ci.yml             # fmt, clippy, build, test
│   ├── ISSUE_TEMPLATE/{bug,feature,model}_*.md
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── dependabot.yml
├── crates/
│   ├── qnn-sys/                     # raw FFI to QNN, bindgen-generated
│   │   ├── Cargo.toml               # NB: doctest = false; no `links` key
│   │   ├── build.rs                 # graceful no-op when QNN_SDK_ROOT unset
│   │   └── src/lib.rs               # include!(bindings.rs)
│   ├── qnn/                         # safe wrapper (shell only — Phase 1 work)
│   ├── npurun-core/                 # engine, manifest, sampler
│   │   └── src/{lib,engine,manifest,sampler}.rs
│   ├── npurun-registry/             # model pull/list/cache
│   ├── npurun-server/               # axum HTTP server (OpenAI + Ollama compat)
│   └── npurun-cli/                  # `npurun` binary (clap)
├── python/
│   ├── npu-convert/                 # x64-Python conversion sidecar (Phase 5)
│   ├── .venv-x64/                   # x64 venv with qai-hub-models  (gitignored)
│   └── .venv-qaihub/                # earlier ARM64 venv attempt   (gitignored)
├── scripts/
│   ├── setup-qnn.ps1                # validates a QAIRT install
│   ├── dev-shell.bat                # vcvarsall arm64 + LLVM + cargo wrapper
│   ├── dev-shell.ps1                # same idea, PowerShell version (if present)
│   ├── smoke_phase0.py              # ORT-EP smoke test (currently unused — bundle is Genie)
│   ├── qai-hub-status.py            # list AI Hub jobs / models
│   └── genie-run.ps1                # Phase 0 NPU smoke test wrapper
├── docs/
│   ├── architecture.md
│   ├── compatibility.md
│   ├── troubleshooting.md
│   └── handoff.md                   # this file
└── target/                          # cargo build output (gitignored)
```

### Files that live OUTSIDE the repo (intentionally)

| Path | Size | Purpose | Status |
|---|---|---|---|
| `C:\AAA\Personal\AI\qairt\2.45.0\` | ~4 GB | Qualcomm QAIRT SDK (proprietary, not redistributable) | Required at runtime |
| `C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite\` | 4.6 GB | The verified working NPU bundle (6 .bin shards + tokenizer.json + genie_config.json + htp_backend_ext_config.json) | Use as-is |
| `C:\AAA\Personal\AI\models\qwen-2.5-7b\onnx\` | 58 GB | Intermediate ONNX export (the .data file alone is 30 GB) | **Safe to delete** — not needed once bundle exists |
| `C:\Users\Brenden\.qaihm\qai-hub-models\models\qwen2_5_7b_instruct\v2\` | varies | qai-hub-models source-weight cache (15 GB zip already extracted) | Keep if planning to re-export |
| `C:\Users\Brenden\.qaihm\qai-hub-models\models\qwen2_5_7b_instruct_w8a16\v2\model_cache.yaml` | tiny | **Pre-populated** with 6 hub-side model IDs so re-exports skip upload | Keep — saves hours |
| `C:\Users\Brenden\.tokens\qai-hub.txt` | 40 bytes | Qualcomm API token | **TODO: rotate** (was pasted in chat) |
| `C:\Users\Brenden\.qai_hub\client.ini` | tiny | qai-hub configured with the token | regenerate after rotation: `qai-hub configure --api_token <new>` |

---

## Working commands (cheat sheet)

### Build / test the Rust workspace

```bat
:: Always invoke cargo via dev-shell.bat — it loads vcvarsall arm64 and LLVM.
:: Running cargo from MSYS bash directly will pick up the wrong link.exe.
scripts\dev-shell.bat cargo build --workspace
scripts\dev-shell.bat cargo build --release --workspace
scripts\dev-shell.bat cargo test --workspace
scripts\dev-shell.bat cargo fmt --all -- --check
scripts\dev-shell.bat cargo clippy --workspace --all-targets -- -D warnings
```

**Default-members exclude `qnn-sys` and `qnn`**; building those requires
`QNN_SDK_ROOT` set. If the workspace is built without QNN, qnn-sys emits
stub bindings so the rest still compiles cleanly.

### Run the Phase 0 NPU smoke test

```powershell
pwsh -File scripts\genie-run.ps1 -Prompt "Hello"
# Custom bundle:
pwsh -File scripts\genie-run.ps1 -Bundle "C:\path\to\bundle" -Prompt "Hello"
```

The script sets `QAIRT_HOME`, prepends QAIRT bin/lib to `PATH`, sets
`ADSP_LIBRARY_PATH=...\lib\hexagon-v73\unsigned`, then runs
`genie-t2t-run.exe -c genie_config.json -p <wrapped-prompt>`.

### Re-export the model from Qualcomm AI Hub

```bat
:: Activate the x64 venv first
call python\.venv-x64\Scripts\activate.bat

:: ALWAYS set these — Genie's status printer crashes on cp1252 console otherwise
set PYTHONIOENCODING=utf-8
set PYTHONUTF8=1

:: ALWAYS use --model-cache-mode enable so already-uploaded shards are
:: reused. Skipping this re-uploads ~30 GB.
python -X utf8 -m qai_hub_models.models.qwen2_5_7b_instruct.export ^
    --device "Snapdragon X Elite CRD" --device-os 11 ^
    --output-dir "C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle" ^
    --onnx-export-dir "C:\AAA\Personal\AI\models\qwen-2.5-7b\onnx" ^
    --sequence-length 128 ^
    --skip-profiling --skip-inferencing ^
    --model-cache-mode enable --synchronous
```

### Run npurun (the CLI we're building)

```bat
scripts\dev-shell.bat cargo run --release -- --help
scripts\dev-shell.bat cargo run --release -- list
scripts\dev-shell.bat cargo run --release -- serve --bind 127.0.0.1:11435
:: Then in another terminal:
curl http://127.0.0.1:11435/v1/models
curl http://127.0.0.1:11435/api/tags
```

The CLI subcommands `pull`, `run`, `rm`, `show`, `ps` print
"Phase X — not yet implemented" until later phases land.

### Inspect Qualcomm AI Hub state

```bat
call python\.venv-x64\Scripts\activate.bat
set PYTHONIOENCODING=utf-8
set PYTHONUTF8=1
python scripts\qai-hub-status.py
```

Lists recent compile/link jobs and uploaded models on the user's account.

---

## Non-obvious gotchas (the long-tail of pain points we've solved)

These are documented in `docs/troubleshooting.md`. Quick-reference:

1. **GNU `link.exe` vs MSVC `link.exe`.** MSYS2 bash ships
   `/usr/bin/link.exe` (GNU coreutils) which shadows MSVC's linker. Symptom:
   `link: extra operand`. **Fix:** always run cargo via `scripts\dev-shell.bat`.

2. **`ring` v0.17 needs clang for ARM64 asm.** Pulled in by `reqwest` →
   `rustls`. **Fix:** install LLVM (`winget install LLVM.LLVM`); dev-shell
   adds `LLVM\bin` to PATH and sets `LIBCLANG_PATH`.

3. **`ort` rc.12 has a bug referencing `SessionOptionsAppendExecutionProvider_VitisAI`.**
   **Fix:** pin `ort = "=2.0.0-rc.10"` in workspace `Cargo.toml`.

4. **`QnnSystem.lib` doesn't ship.** QAIRT 2.45 ships `QnnSystem.dll` only —
   no static import library. **Fix:** `qnn-sys/Cargo.toml` does **not** set
   `links`, and `build.rs` does **not** emit `cargo:rustc-link-lib=dylib=QnnSystem`.
   Phase 1 will load symbols at runtime via the `libloading` crate.

5. **bindgen-generated doc comments break rustdoc.** QNN headers contain
   doc comments with markdown bullet points and "function"/"call" words that
   rustdoc tries to compile as Rust. **Fix:** `[lib] doctest = false` in
   `qnn-sys/Cargo.toml`.

6. **Windows console can't print Unicode emoji.** qai-hub's status animation
   prints `⏳`, the default cp1252 encoding crashes. **Fix:** always set
   `PYTHONIOENCODING=utf-8` and `PYTHONUTF8=1` before running the export.

7. **The export pipeline DOES upload to Qualcomm's cloud.** The compile job
   runs on real Snapdragon X Elite hardware in their farm. Without `--model-cache-mode enable`
   and a populated cache, you upload ~30 GB to do this. We pre-populated
   `~/.qaihm/qai-hub-models/models/qwen2_5_7b_instruct_w8a16/v2/model_cache.yaml`
   with the 6 shard model IDs from a prior partial run, which is how the
   verified export skipped uploads entirely.

8. **`--model-cache-mode enable` only reuses cache entries written by prior
   `enable` runs, not the default `disable`.** First run with `disable` ->
   no cache entries -> next run with `enable` re-uploads. Either pre-write
   the cache yaml, or always start with `enable`.

9. **AI Hub is a build service, not a model store.** There's no "Download"
   button on the website for most models. The only path is the qai-hub-models
   Python tool, which uploads + compiles + downloads the result.

10. **All NPU-pre-built LLMs on AI Hub are capped at 4096-token context.**
    Hardware constraint (per-graph compile, ~1.3 GB shared LPDDR5x ceiling).
    Cannot be bypassed without recompiling, which requires Qualcomm's
    proprietary toolchain. Snapdragon X2 (~80 TOPS, mid-2026) may relax this.

11. **The export crashed silently mid-upload twice during the verified run.**
    Both times the Python process disappeared from the process table with no
    error. Cause unclear (network glitch? Windows resource pressure?).
    The retry path that worked: pre-populate the cache yaml so retries don't
    re-upload, then re-run with `--model-cache-mode enable`.

12. **The Llama family on AI Hub requires HuggingFace authentication +
    Meta's license click-through.** Qwen 2.5 7B is Apache 2.0 and ungated —
    use it for smoke tests / development to avoid the auth grind.

---

## Open immediate items (TODO list at handoff time)

Listed in priority order:

1. **Rotate the Qualcomm AI Hub API token.** It was pasted directly in chat
   so it's in this transcript: `k17t7w98px69lpagjx9cf8q7lydbfo6sxcrkiouz`.
   Workflow: <https://workbench.aihub.qualcomm.com/> → Account → Settings →
   API Token → revoke + create new → save to `~/.tokens/qai-hub.txt` →
   `qai-hub configure --api_token <new>`.

2. **Optional cleanup:** delete `C:\AAA\Personal\AI\models\qwen-2.5-7b\onnx\`
   (58 GB of intermediate ONNX no longer needed; the bundle has everything).

3. **Phase 1: build `qnn-sys` + `qnn` crates.** The bindings build now via
   bindgen against `$QNN_SDK_ROOT/include/QNN/*.h`. Phase 1 wraps them in a
   safe Rust API, with runtime `libloading` to avoid the missing-`.lib`
   problem. Definition of done: `cargo test` loads a context binary and
   runs a forward pass on the NPU, output bit-matches `genie-t2t-run.exe`.

4. **Phase 2: `npurun-core` real inference loop.** Wraps Genie's C API
   (since the AI Hub LLM toolchain only emits Genie bundles) instead of
   ORT. Tokenizer + sampler + KV cache loop, Qwen 2.5 7B hardcoded.
   Definition of done: `cargo run --example qwen` streams coherent tokens
   from NPU at ≥5 tok/s steady-state, NPU column shows >0% sustained.

5. **Phase 3+:** CLI fleshed out, HTTP server made real, npu-convert pipeline,
   release prep. See plan file for details.

---

## Architecture pivot from the original plan

The plan assumed the default inference path would be **ONNX Runtime + QNN
Execution Provider** (via the `ort` crate), with Genie as a feature-flagged
secondary path. Phase 0 demonstrated that **the qai-hub-models export only
emits Genie bundles for LLMs** (`target_runtime: qnn_dlc`, not ONNX). So the
priorities flip:

- **Phase 1+ inference path:** Genie via libloading. The `qnn-sys` crate
  still exposes raw QNN bindings for low-level work and non-LLM models;
  the LLM path goes through Genie.
- **Phase 5 (`npu-convert`):** still useful for non-LLM models or for taking
  control of the conversion pipeline ourselves rather than relying on AI Hub.

---

## Verifying you have a working setup

Run these in order. If any step fails, see `docs/troubleshooting.md`.

```bat
:: 1. SDK validator
pwsh -File scripts\setup-qnn.ps1
:: Expected: all-green; "All checks passed"

:: 2. Workspace builds and tests pass
scripts\dev-shell.bat cargo build --workspace
scripts\dev-shell.bat cargo test --workspace
:: Expected: 25/25 tests pass

:: 3. Phase 0 NPU smoke test
pwsh -File scripts\genie-run.ps1 -Prompt "Tell me a one-line joke."
:: Expected: text appears between [BEGIN]: and [END]
:: Open Task Manager > Performance > NPU; should see >5% utilization
```

If all three pass, the laptop is in the same state ours was in when this
handoff was written.

---

## Where to find more

- **Plan:** `C:\Users\Brenden\.claude\plans\there-currently-exists-no-parallel-sparrow.md`
- **Memory index:** `C:\Users\Brenden\.claude\projects\c--AAA-Personal-AI\memory\MEMORY.md`
- **Project memory:** `…\memory\project_npurun.md` (Phase 0 status snapshot)
- **Feedback memory:** `…\memory\feedback_robustness.md`, `feedback_plain_language.md`
- **Reference:** `…\memory\reference_npu_ecosystem.md` (snapshot of NPU ecosystem April 2026)

The plan is the durable design doc. Memory files capture user preferences
and frozen-in-time facts. This handoff doc is the operational state.

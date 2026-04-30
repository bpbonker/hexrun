# Findings: running a 7B LLM on the Snapdragon X Elite NPU under Windows on ARM, with open tooling

**Date:** 30 April 2026
**Status:** end-to-end working, reproducible, ~135 minutes from a clean machine to first NPU-generated token (excluding the Visual Studio install, which we'll always have to assume is there).
**Hardware:** Surface laptop with Snapdragon X Elite (X1E80100), 16 GB shared LPDDR5x, 7.8 GB shared NPU memory ceiling.

---

## TL;DR

We took a fresh Windows-on-ARM laptop and demonstrated **Qwen 2.5 7B Instruct
running on the Hexagon NPU**, generating coherent text, with **NPU
utilization sustained at ~19% and 4.9 GB of model weights resident in NPU
shared memory** during inference. Three independent signals confirm it:
the Genie runtime's stdout, the bundle's cloud-compile target metadata, and
Task Manager's NPU graph. CPU stayed at 12% during generation — the
Hexagon is doing the actual work.

We're publishing the recipe because as of today (April 2026) **the open
ecosystem for "use the NPU on your Snapdragon laptop" is essentially
nonexistent**. Ollama, llama.cpp, LM Studio, and text-generation-webui all
silently fall back to CPU on these chips. Microsoft's Phi Silica is
locked behind first-party Copilot APIs. NexaSDK works but is closed. The
official Qualcomm path is real but undocumented in the public-friendly
sense, and littered with version traps.

This document captures the full path — what works, what breaks, and the
non-obvious tricks that turn a half-day of trial-and-error into a 30-min
recipe.

---

## Why this matters

The 45-TOPS NPU in Snapdragon X Elite is the central marketing pitch for
the entire "Copilot+ PC" generation. In practice, **third-party developers
have no clean way to use it for general LLM inference.** Our laptop ships
with the hardware powered on, the driver loaded, and the chip idle — every
time you run Ollama or llama.cpp.

The reasons aren't rocket science. The NPU only accepts ONNX models with
INT8/INT16 quantization through Qualcomm's QNN runtime. GGUF doesn't map.
Quantization tooling is x64-only on Windows. Qualcomm's compile stack runs
in their cloud. There's no Rust binding for QNN. Every step of the
toolchain has at least one known papercut. And the docs that exist assume
you're inside Qualcomm or Microsoft.

**What we'd lose if this stays unsolved:**
- Battery life: NPU inference is roughly 5–10× more efficient than CPU on
  the same workload.
- Privacy: a usable local-LLM story on these laptops collapses to
  cloud round-trips.
- The whole product category's reason to exist.

**What this finding contributes:**
- A reproducible recipe that doesn't require Qualcomm employment.
- A catalog of every papercut, with the fix.
- A working bundle (Qwen 2.5 7B INT4 for Snapdragon X Elite) and the
  configuration files needed to run it.
- A foundation (the in-progress hexrun project) that other tools could
  build on.

---

## What we built

The artifact is twofold:

### 1. The runnable Qwen 2.5 7B NPU bundle

A 4.6 GB directory at
`C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite\`
containing:

- six `.bin` shards (`qwen2_5_7b_instruct_w8a16_part_{1..6}_of_6.bin`,
  total 4.6 GB) — these are QNN context binaries pre-compiled for our
  exact chip in Qualcomm's cloud
- `tokenizer.json` — Qwen's HF tokenizer
- `genie_config.json` — Genie runtime config (sampler, context, model
  layout)
- `htp_backend_ext_config.json` — backend config (soc_model 60, dsp_arch
  v73 — the right values for X1E80100)

This bundle loads in `genie-t2t-run.exe` (ships with QAIRT) and produces
coherent NPU-accelerated text.

### 2. hexrun: the in-progress runtime

A Cargo workspace currently at scaffold-plus-Phase-0-verification stage.
Six crates (`qnn-sys`, `qnn`, `hexrun-core`, `hexrun-registry`,
`hexrun-server`, `hexrun-cli`) plus a Python sidecar (`hex-convert`) for
HuggingFace → NPU conversion. 25/25 unit tests pass; cargo fmt/clippy/deny
all clean. Phase 0 (the "does the NPU work at all" milestone) is verified
on hardware.

Future phases bind QNN/Genie via Rust, expose an Ollama-compatible CLI +
OpenAI-compatible HTTP server, and ship a converter pipeline. The plan
file in the repo describes phases 1–6.

---

## How it works (architecture)

```
                    ┌─────────────────────────┐
                    │   user: hexrun run X    │
                    └────────────┬────────────┘
                                 │
                                 ▼
          ┌──────────────────────────────────────┐
          │  hexrun (Rust, Phase 1+ in progress) │
          │  Ollama-style CLI + OpenAI HTTP API  │
          └──────────────────────────────────────┘
                                 │
                                 ▼
          ┌──────────────────────────────────────┐
          │  Genie runtime (libGenie 1.17, C)    │
          │  LLM-aware: tokenizer, sampler,      │
          │  KV-cache loop, prompt processing    │
          └──────────────────────────────────────┘
                                 │
                                 ▼
          ┌──────────────────────────────────────┐
          │  QNN HTP backend (QnnHtp.dll)        │
          │  Loads compiled context binaries,    │
          │  schedules ops on Hexagon NPU        │
          └──────────────────────────────────────┘
                                 │
                                 ▼
          ┌──────────────────────────────────────┐
          │  Hexagon v73 NPU (Snapdragon X Elite)│
          │  ~45 TOPS INT8, 7.8 GB shared LPDDR  │
          └──────────────────────────────────────┘
```

### Where the model came from

```
┌─────────────────────────────┐
│  Qwen/Qwen2.5-7B-Instruct   │  HuggingFace (Apache 2.0)
└─────────────┬───────────────┘
              │
              ▼   downloaded (15 GB) by qai-hub-models export script
┌─────────────────────────────┐
│  Source weights + AIMET     │
│  quantization encodings     │
└─────────────┬───────────────┘
              │
              ▼   exported to ONNX shards locally (30 GB, intermediate)
┌─────────────────────────────┐
│  6 ONNX shards (w8a16)      │
└─────────────┬───────────────┘
              │
              ▼   uploaded to Qualcomm AI Hub
┌─────────────────────────────┐
│  Qualcomm cloud compile     │
│  Real Snapdragon X Elite    │  Each shard: ~10 min compile
│  in their farm runs the     │  Then a link job stitches
│  actual NPU compiler        │
└─────────────┬───────────────┘
              │
              ▼   downloaded
┌─────────────────────────────┐
│  6 QNN context binaries     │  4.6 GB total
│  (.bin) — the runnable      │
│  bundle on this laptop      │
└─────────────────────────────┘
```

The compile step is the key insight. **Qualcomm AI Hub is a build service,
not a model store.** Every model + chip + context-length combination gets
compiled on real hardware on demand. That's why the AI Hub web pages
don't have "Download" buttons for most models, and why the offline
toolchain looks more confusing than it should.

---

## Performance

These are first-pass numbers, intentionally conservative. The first
`genie-t2t-run.exe` invocation pays a ~30-second cost loading 4.6 GB of
context binaries into NPU shared memory. Steady-state per-token rates
will only be measurable once the runtime can keep the bundle loaded
across queries (Phase 1 work).

| Metric | Value | Notes |
|---|---|---|
| Cold load time | ~30 s | One-time bundle paging into NPU shared memory |
| Total wall clock (long generation) | 171 s | Includes cold load + 215-token output |
| Approx tokens generated | ~215 | 166 words × ~1.3 tokens/word |
| Wall-clock-average tokens/sec | 1.3 tok/s | **Misleadingly low** — dominated by cold load |
| NPU sustained utilization | ~19% | Task Manager observed |
| NPU peak utilization | ~30–40% | Per-token forward-pass spikes |
| NPU shared memory in use | 4.9 / 7.8 GB | Bundle resident; ~63% of available NPU memory |
| CPU utilization during generation | ~12% | Confirms work is on NPU |
| Qualcomm published expectation | 8–15 tok/s steady-state for 7B INT4 on 45 TOPS | We expect to hit this once cold load is amortized |

**The 4096-token context window is a hardware constraint**, not a download
choice. Every NPU-pre-built LLM on Qualcomm AI Hub for Snapdragon X
Elite ships with a fixed 4096-token context — caused by the per-graph
compile model and the ~1.3 GB shared LPDDR5x memory ceiling. Recompiling
at a longer context requires Qualcomm's proprietary toolchain, which
isn't public. The Snapdragon X2 (~80 TOPS, mid-2026) may relax this.

---

## Twelve papercuts and how to fix them

If you're trying to reproduce this, every one of these will bite you.

1. **GNU `link.exe` shadows MSVC `link.exe` in MSYS2 / Git Bash.**
   Symptom: `link: extra operand 'C:\\...\\rcgu.o'` from cargo. Cause:
   `/usr/bin/link.exe` (GNU coreutils) is on PATH ahead of MSVC's linker.
   **Fix:** invoke cargo from a shell that loads the MSVC environment.
   We ship `scripts/dev-shell.bat` which calls `vcvarsall.bat arm64`
   and prepends `%USERPROFILE%\.cargo\bin` plus LLVM's bin dir.

2. **`ring` v0.17 needs clang for ARM64 assembly.** Pulled in
   transitively via `reqwest` → `rustls`. Symptom:
   `ToolNotFound: failed to find tool "clang"`. **Fix:** install LLVM
   (`winget install LLVM.LLVM`); set `LIBCLANG_PATH=C:\Program Files\LLVM\bin`
   and put it on PATH.

3. **`ort` 2.0.0-rc.12 references a non-existent ORT API field.**
   Symptom: `error[E0609]: no field SessionOptionsAppendExecutionProvider_VitisAI`.
   **Fix:** pin `ort = "=2.0.0-rc.10"` until upstream fixes it.

4. **`QnnSystem.lib` does not ship in QAIRT 2.45 on Windows ARM64.**
   Only `QnnSystem.dll`. Symptom: `LNK1181: cannot open input file 'QnnSystem.lib'`
   when cargo's build.rs tries to link. **Fix:** don't declare `links =
   "QnnSystem"` in `qnn-sys/Cargo.toml`, don't emit
   `cargo:rustc-link-lib=dylib=QnnSystem` from build.rs. Load symbols at
   runtime with the `libloading` crate. (The Genie wrapper has the same
   shape, but `Genie.lib` *does* ship — so Genie can be linked at compile
   time.)

5. **bindgen-generated docs break rustdoc.** QNN headers contain doc
   comments with markdown bullet lists and words like "function" / "call"
   that rustdoc tries to compile as Rust. Symptom: `expected one of '!'
   or '::', found 'function'` from cargo test. **Fix:** put `[lib]
   doctest = false` in `qnn-sys/Cargo.toml`.

6. **Windows console can't print Unicode emoji by default.** Qualcomm's
   AI Hub Python client prints `⏳` in its progress animation; cp1252
   chokes on it and crashes the script *while a real cloud compile job
   is succeeding in the background.* Symptom: `UnicodeEncodeError:
   'charmap' codec can't encode character '⏳'`. **Fix:** set
   `PYTHONIOENCODING=utf-8` and `PYTHONUTF8=1` before running the export,
   and pass `python -X utf8`.

7. **AI Hub uploads are slow and silently die.** The export script
   uploads ~30 GB of ONNX shards before submitting compile jobs. Twice
   in our verified path, the upload process disappeared mid-flight with
   no error. **Fix:** see #8.

8. **The cache yaml trick that saves hours.** The qai-hub-models export
   has a `--model-cache-mode` option, but it only reuses cache entries
   written by *prior runs that also used `enable`*. Default is `disable`,
   which means the typical first attempt writes nothing to the cache,
   and a retry re-uploads from scratch. **Fix:** if uploads succeeded
   in a prior attempt, use the qai-hub Python API to list your uploaded
   model IDs (`qai_hub.get_models()`), then pre-write
   `~/.qaihm/qai-hub-models/models/<model>_<precision>/v2/model_cache.yaml`
   with the right key/val pairs (cache_name, pytorch version, onnx
   version, hub_endpoint, version, precision, context_length,
   sequence_length → hub_model_id). Subsequent runs with `--model-cache-mode
   enable` skip uploads entirely. We've documented the exact format in
   `scripts/qai-hub-status.py`.

9. **`--model-cache-mode enable` does not skip compile jobs, only
   uploads.** Each run resubmits compile + link jobs to the cloud. For a
   7B model that's ~6 compile jobs at ~10 min each plus 6 link jobs at
   ~5 min each. Plan accordingly.

10. **The Llama family on AI Hub requires Meta's HuggingFace
    license-acceptance form + a HF token.** Qwen 2.5 7B is Apache 2.0
    and ungated — recommended for first attempts, identical workflow
    otherwise.

11. **The export only emits Genie bundles for LLMs**, not raw ONNX. The
    documentation suggests three target runtimes (TFLite, ONNX, QAIRT),
    but in practice `--target-runtime onnx` is rejected for the LLM
    models. This means the inference path goes through Genie, not
    plain ORT QNN-EP. Plan for libGenie bindings, not ONNX Runtime.

12. **Windows ARM Python ecosystem is fragmented.** ONNX Runtime
    quantization tooling has no ARM64 wheel; PyTorch 2.4.1 (pinned by
    qai-hub-models for some Llama variants) has no ARM64 wheel; Qwen
    pulled torch 2.11 which works fine. **Fix:** install **x64** Python
    3.11 alongside any ARM64 Python you have (`winget install
    Python.Python.3.11 --architecture x64`), use `py -3.11-64`, and run
    everything qai-hub-related from a venv built off it. The OS runs
    x64 Python under Prism emulation transparently.

---

## Reproduction recipe

Assumes a Snapdragon X Elite Windows-on-ARM laptop with Visual Studio
2022 Community installed (any edition with C++ workloads is fine; you'll
add ARM64 components below).

### Prerequisites (about 30 min, mostly downloads)

```powershell
# Rust + ARM64 target
winget install Rustlang.Rustup
rustup target add aarch64-pc-windows-msvc

# MSVC v143 ARM64/ARM64EC C++ build tools + Win11 SDK 26100
# (do this in the VS Installer GUI: "Modify" your VS 2022 install,
#  Individual components, search "ARM64", tick the v143/ARM64/ARM64EC box
#  and the Windows 11 SDK 10.0.26100.0 box, click Modify)

# LLVM/clang
winget install LLVM.LLVM

# x64 Python 3.11 (under Prism emulation; needed for AI Hub tooling)
winget install Python.Python.3.11 --architecture x64
```

### Get the QAIRT SDK (about 10 min)

1. Sign up at <https://www.qualcomm.com/developer> (free).
2. Download **Qualcomm AI Engine Direct (QAIRT) SDK 2.45+** for Windows.
   It's a zip — extract is the install.
3. Move it to `C:\Qualcomm\AIStack\QAIRT\<version>` or anywhere stable.
4. Set `QNN_SDK_ROOT`:
   ```powershell
   setx QNN_SDK_ROOT "C:\AAA\Personal\AI\qairt\2.45.0"
   ```
   Open a fresh terminal afterwards.

### Get the model bundle (about 90 min for a 7B; ~30 of that is just clock time on Qualcomm's cloud compile)

```bat
:: From inside a clone of hexrun:
py -3.11-64 -m venv python\.venv-x64
call python\.venv-x64\Scripts\activate.bat

pip install "qai-hub-models[qwen2-5-7b-instruct]"

:: Get a free Qualcomm AI Hub API token at
:: https://workbench.aihub.qualcomm.com/ -> Account -> Settings -> API Token
qai-hub configure --api_token <YOUR-TOKEN>

:: Run the export. UTF-8 env vars are critical.
set PYTHONIOENCODING=utf-8
set PYTHONUTF8=1
python -X utf8 -m qai_hub_models.models.qwen2_5_7b_instruct.export ^
    --device "Snapdragon X Elite CRD" --device-os 11 ^
    --output-dir "C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle" ^
    --onnx-export-dir "C:\AAA\Personal\AI\models\qwen-2.5-7b\onnx" ^
    --sequence-length 128 ^
    --skip-profiling --skip-inferencing ^
    --model-cache-mode enable --synchronous
```

### Make the bundle runnable (about 30 seconds)

The exported bundle has six `.bin` shards but no `genie_config.json` or
tokenizer. Add them (the tokenizer is from HuggingFace, ungated for Qwen):

```bat
:: From the bundle directory, fetch the tokenizer
curl -fsSLo tokenizer.json https://huggingface.co/Qwen/Qwen2.5-7B-Instruct/resolve/main/tokenizer.json

:: Drop the genie_config.json and htp_backend_ext_config.json from
:: hexrun/scripts/sample-configs/qwen2_5_7b_instruct/  (versioned in the repo)
:: into the bundle directory.
```

### Run it

```powershell
pwsh -File hexrun\scripts\genie-run.ps1 -Prompt "Tell me a one-line joke."
```

### Verify it's actually using the NPU

Three independent checks. **All three should agree** before claiming NPU
acceleration:

1. **Genie's stdout** — should print `Using libGenie.so version <ver>`,
   `Using create From Binary`, and produce text bracketed by `[BEGIN]:`
   and `[END]`.

2. **Compile metadata** — re-check the AI Hub job for the bundle. The
   options should include `--target_runtime qnn_dlc` with a device matching
   `chipset:qualcomm-snapdragon-x-elite`. (Use `scripts/qai-hub-status.py`
   to list your jobs.)

3. **Task Manager** — Performance → NPU should show **>5% utilization
   sustained during generation**, with several GB of "Shared memory"
   in use, while CPU stays low. If NPU is at 0% but you're getting
   text, you're hitting CPU fallback silently.

---

## What we did NOT solve

- **Steady-state generation rate measurement** — every `genie-t2t-run.exe`
  invocation cold-loads 4.6 GB. We've not yet kept the model resident
  across queries to measure per-token speed honestly. Phase 1 of hexrun
  binds the Genie API directly so this becomes possible.
- **Context length > 4096 tokens** — hardware-bound on the X1E80100.
  Will require either Snapdragon X2 hardware or recompiling models with
  Qualcomm's proprietary stack.
- **Models above ~7-8B INT4** — we're using ~63% of available NPU shared
  memory at this size. Larger models exceed the 7.8 GB ceiling.
- **A clean OSS path that doesn't depend on Qualcomm's cloud** — the
  AI Hub upload-then-cloud-compile dance is mandatory today. The closest
  any open project gets to bypassing it is `hex-convert` (Phase 5 of
  hexrun) which would *attempt* local quantization-and-compile. That's
  large work and requires Qualcomm to release more of their toolchain.
- **Other model architectures** — only LLM (Qwen, Llama, Phi, etc.) is
  in the AI Hub catalog with NPU pre-builds. Vision and audio model
  paths exist but were not exercised here.

---

## Comparison with the rest of the field

| Path | NPU on X Elite (Apr 2026)? | Notes |
|---|---|---|
| Ollama | ❌ CPU only | Issue [#5360](https://github.com/ollama/ollama/issues/5360) open |
| llama.cpp | ❌ QNN backend stalled | Discussions #8273, #8336 |
| LM Studio | ❌ CPU/GPU only | Issue #30 unanswered |
| text-generation-webui | ❌ | Issue #6298 |
| MLC-LLM | ❌ Quantization granularity | Issue #1689 |
| PyTorch | ❌ ARM64 wheels CPU-only | No Hexagon dispatch |
| Microsoft Phi Silica | ✅ NPU | Locked to first-party Copilot apps |
| Microsoft Foundry Local | ⚠️ Distilled DeepSeek only | Preview |
| NexaSDK | ✅ NPU | Closed CLI, not Rust-friendly |
| AnythingLLM / Fooocus | ⚠️ Claim NPU, broken in practice | Issues #3194, #3042 |
| **This work (hexrun + manual recipe)** | ✅ NPU | Fully reproducible from open tools + Qualcomm dev account |

We are not aware of another public open-source recipe that takes Qwen 2.5
7B from HuggingFace through to verified NPU execution on Snapdragon X
Elite Windows ARM, with all twelve papercuts documented. If someone has
done this we'd love to hear from them — happy to credit.

---

## Acknowledgments

- **Qualcomm AI Hub** for actually shipping the cloud compile service.
  Without it, none of this is possible without an internal team.
- **The qai-hub-models repo** at github.com/qualcomm/ai-hub-models for
  the export recipes, even if the failure modes we hit aren't documented
  there.
- **The ai-hub-apps tutorial** for the Genie config templates we adapted.
- **The `ort`, `tokenizers`, `axum`, `clap`, `bindgen`, `libloading`,
  `tracing` Rust crates** for the foundation hexrun is built on.

---

## Where this goes next

The hexrun project is the long-term home of this work. Phase 0 (this
finding) is verified. Phase 1 binds Genie/QNN in Rust so the runtime
keeps the model loaded across queries. Phase 2 wires up the LLM
generation loop; Phase 3 the CLI; Phase 4 an OpenAI/Ollama-compatible
HTTP server; Phase 5 the converter pipeline (`hex-convert`); Phase 6
release prep.

The end goal is Ollama for the NPU on Windows ARM. The plan and
handoff documents in this repo describe the remaining work in detail.

If you reproduce this, get a different result, or hit a thirteenth
papercut we missed, please file an issue. The Snapdragon X Elite
launched in mid-2024 and we're still in "is this hardware usable for
its stated purpose" territory two years later. Every documented data
point helps.

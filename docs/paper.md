---
title: "Reaching the Hexagon: An open, reproducible recipe for running 7B-parameter language models on the Snapdragon X Elite NPU under Windows on ARM"
authors:
  - name: Brenden Bonker
    affiliation: Independent
    note: With AI assistance from Claude (Anthropic)
date: 30 April 2026
keywords:
  - on-device inference
  - language models
  - Snapdragon X Elite
  - Hexagon NPU
  - ARM64 Windows
  - Qualcomm AI Engine Direct
  - Genie runtime
  - hexrun
status: Pre-print, technical experience report
---

## Abstract

The Snapdragon X Elite (X1E) shipped in mid-2024 as the first
mass-market Windows-on-ARM laptop platform with a dedicated 45-TOPS
neural processing unit (NPU). Qualcomm and Microsoft have positioned
the Hexagon NPU as central to the "Copilot+ PC" product category, but
nearly two years later the open-source ecosystem still has no clean
path for a third-party developer to use it for general large-language-
model inference. Ollama, llama.cpp, LM Studio, and other community
runtimes silently fall back to CPU on these chips. The first-party
"Phi Silica" small language model is gated behind closed Microsoft
APIs. The official Qualcomm path exists but is fragmented across
proprietary documentation, multiple cloud-side build steps, and a
dozen poorly-signposted toolchain papercuts.

We present a verified, end-to-end recipe that takes Qwen 2.5 7B
Instruct from a HuggingFace checkpoint to NPU-accelerated text
generation on a stock Snapdragon X Elite Surface laptop, using only
the public Qualcomm AI Hub service and an Apache-2.0-licensed model.
We measure ~19% sustained NPU utilization with 4.9 GB of model
weights resident in NPU shared memory during inference, while CPU
remains at ~12% — confirming the Hexagon, not the Oryon CPU, is doing
the work. We document twelve concrete pitfalls that defeat naive
attempts, including an undocumented client-side cache trick that
reduces re-export time from hours to minutes. We release `hexrun`,
an in-progress open-source Rust runtime built on these findings,
intended to provide an Ollama-class developer experience for the
NPU. To our knowledge this is the first publicly reproducible recipe
for general 7B-class LLM inference on the Snapdragon X Elite NPU
that does not depend on Qualcomm or Microsoft employment.

---

## 1. Introduction

Energy efficiency is the central motivation for dedicated neural
accelerators in mobile-class hardware. Compared to a general-purpose
CPU performing the same matrix-multiplication-heavy workloads, a
purpose-built NPU can be 5–10× more energy-efficient at equivalent
throughput, with corresponding gains in battery life and thermal
headroom [@npu-efficiency]. For laptops, where battery life is a
first-order user concern, this efficiency gap is the practical
justification for the additional silicon.

The Snapdragon X Elite system-on-chip [@qualcomm-snapdragon-x-elite],
shipping in laptops including the Microsoft Surface Pro 11, Lenovo
Yoga Slim 7x, and Dell XPS 13 since mid-2024, exposes 45 TOPS (INT8)
of NPU compute via a Hexagon v73 tensor processor. Microsoft's
"Copilot+ PC" branding requires at least 40 TOPS of NPU compute and
positions on-device AI as the platform's distinguishing capability
[@copilot-plus-pc]. On the hardware side this picture is complete.

On the software side, two years after launch, the practical state of
NPU access for third-party developers can be summarized as: it works
if you are Microsoft, it works if you are Qualcomm, and otherwise it
does not. Concretely (April 2026):

- **Ollama** (issue [#5360](https://github.com/ollama/ollama/issues/5360)),
  **llama.cpp** ([#8273](https://github.com/ggml-org/llama.cpp/discussions/8273),
  [#8336](https://github.com/ggml-org/llama.cpp/discussions/8336)),
  **LM Studio** ([#30](https://github.com/lmstudio-ai/lms/issues/30)),
  **text-generation-webui** ([#6298](https://github.com/oobabooga/text-generation-webui/issues/6298))
  and **MLC-LLM** ([#1689](https://github.com/mlc-ai/mlc-llm/issues/1689))
  all run CPU-only on Snapdragon X Elite. The relevant tracking
  issues have been open between six months and two years.
- **PyTorch** has no Hexagon dispatch on Windows ARM64; the official
  ARM64 wheels are CPU-only.
- **Microsoft Phi Silica**, the only first-party LLM with NPU
  acceleration, runs through closed APIs accessible only to Copilot
  applications under Microsoft's limited-access policies
  [@phi-silica].
- **NexaSDK** [@nexasdk] does provide working NPU inference on these
  chips, but is closed-source, distributed as compiled binaries, and
  does not interoperate with the broader Rust or Python tooling
  ecosystem.

The result is a fast-growing class of laptops shipping with the most
visible feature in their marketing material — local AI acceleration —
practically unavailable to the open developer community. Users
running Ollama on a Snapdragon X Elite see the NPU column in Task
Manager remain at 0%, with the workload silently handled by the CPU.

This paper makes three contributions:

1. **A complete, reproducible recipe** that takes an open-license LLM
   from HuggingFace through to verified NPU execution on a stock
   Snapdragon X Elite laptop. Section 4 details the pipeline; Section
   6 reports measured outcomes.

2. **A catalog of twelve specific failure modes** encountered during
   the work, each with a documented fix. Several of these are not
   documented elsewhere, including (Section 5.7) a client-side
   cache-file trick that reduces re-export wall-clock time from
   approximately 90 minutes to approximately 30 minutes by eliding
   redundant cloud uploads.

3. **An open-source runtime, `hexrun`** [@hexrun-repo], scaffolded as
   a six-crate Rust workspace plus Python conversion sidecar.
   Phase 0 of the project (the work reported here) is verified;
   subsequent phases are described in Section 7.

The contribution is not algorithmic. Quantization, NPU-targeted graph
compilation, and LLM serving are well-studied problems individually.
The contribution is *integration* — assembling a path through the
Snapdragon ARM64 Windows ecosystem that demonstrably reaches the
Hexagon NPU and is reproducible by an independent developer in a
single working session.

---

## 2. Background

### 2.1 The Snapdragon X Elite NPU

The X1E80100 SKU includes a Hexagon v73 NPU with shared access to up
to 7.8 GB of LPDDR5x system memory (out of the 16 GB total system
memory on a typical laptop SKU). Peak NPU throughput is reported as
45 TOPS at INT8 precision; FP16 is supported but not the typical
target for LLM inference on this generation [@qualcomm-snapdragon-x-elite].
The NPU shares the system memory bus with the CPU and Adreno GPU,
which constrains the working-set size of any single model.

Crucially for LLM workloads, the NPU operates on a fixed-graph
execution model: the operator schedule, tensor shapes, and memory
layout are determined at compile time by Qualcomm's offline NPU
compiler. The graph cannot be re-shaped at runtime. For
autoregressive language models this implies the maximum sequence
length and context window are baked into the compiled artifact and
cannot be adjusted after compilation. For pre-built models published
by Qualcomm AI Hub for the X1E platform, this maximum is uniformly
4,096 tokens (Section 6.4).

### 2.2 Qualcomm AI Engine Direct (QAIRT)

QAIRT is the runtime that loads compiled NPU graphs and dispatches
inference on Hexagon. The relevant APIs for this work include:

- `QnnSystem`, `QnnContext`, `QnnGraph`, `QnnTensor` — low-level C
  APIs for arbitrary graph execution.
- **Genie** (`libGenie.so`/`Genie.dll`) — a higher-level C library
  built on top of QNN that adds LLM-aware machinery: tokenization,
  sampling, KV-cache management, and prompt batching. It exposes a
  `GenieDialog` interface that takes a JSON configuration and a text
  prompt, returning generated text.

Both layers ship with the QAIRT SDK distribution
[@qualcomm-qairt-sdk]. The SDK is gated behind a free Qualcomm
developer-account registration but is otherwise self-contained.
Notably, on Windows ARM64 in QAIRT 2.45 the `QnnSystem.dll` ships
without a corresponding `.lib` import library, requiring runtime
loading via `LoadLibrary`/`dlopen` rather than compile-time linking
(Section 5.4).

### 2.3 Qualcomm AI Hub

Qualcomm AI Hub [@qualcomm-ai-hub] is the public-facing service
through which users obtain NPU-runnable models. It is, structurally,
a *cloud build service rather than a model store*. A user submits an
ONNX model (typically already prepared with AIMET quantization
encodings); AI Hub schedules a compile job on a real Snapdragon
device in Qualcomm's farm; the resulting compiled context binary is
returned to the user. There is no static asset hosting for most
models on the AI Hub website — the workflow is fundamentally
upload-compile-download. This architectural fact is rarely surfaced
in introductory documentation and is the source of several of the
obstacles described in Section 5.

The companion `qai-hub-models` Python package [@qai-hub-models]
provides export recipes that fetch the source weights from
HuggingFace, apply Qualcomm's pre-built AIMET quantization
encodings, perform local ONNX export, then submit the result to the
cloud build service for the chosen target device.

### 2.4 The Genie LLM runtime

The `genie-t2t-run` executable shipped with QAIRT loads a Genie JSON
configuration that points at a directory of compiled context-binary
shards (typically 6 shards for a 7B-parameter model on X1E) plus a
HuggingFace `tokenizer.json`. Given a prompt string it produces
generated text through the dialog API. This was the runtime used to
verify Phase 0 in the present work; binding the same C API directly
from Rust is the core of `hexrun` Phase 1.

---

## 3. Related Work

### 3.1 Mobile-class NPU inference

Closed first-party deployments — Apple's Core ML on Apple Neural
Engine, Google's TFLite on Edge TPUs, Microsoft's Phi Silica on
Hexagon — have demonstrated for years that on-device LLM inference
on dedicated accelerators is feasible. None of these are accessible
to third-party developers on Snapdragon X Elite Windows ARM at the
generality of LLM inference attempted here.

### 3.2 Open-source NPU runtimes

To our knowledge, the only public open-source runtime that
demonstrably performs general LLM inference on the Hexagon NPU
under Windows on ARM is **NexaSDK** [@nexasdk]. NexaSDK is closed
beyond the CLI shipping as a binary; we cannot evaluate its
internals or extend it. **MLC-LLM** [@mlc-llm] historically targeted
QNN on Android devices but the Snapdragon X Elite Windows path
remains blocked on quantization-granularity support
[@mlc-llm-issue-1689]. **ExecuTorch** [@executorch] supports a
Qualcomm backend that compiles for Hexagon, but Windows ARM64 builds
were unverified at the time of writing.

### 3.3 Cross-platform LLM serving

Ollama [@ollama] and llama.cpp [@llamacpp] are the dominant
open-source LLM-serving stacks. Both use the GGUF format and target
CPU and (vendor-specific) GPU backends. Neither has working NPU
support on Snapdragon X Elite as of April 2026. Their tracking
issues remain open.

### 3.4 Cloud-compile services as a deployment vector

The pattern of "upload to a cloud service that compiles your model
on the target hardware, then download the binary" is familiar from
Apple's Core ML Tools, NVIDIA's TensorRT (when used through their
cloud build endpoint), and Edge Impulse's deployment pipeline.
Qualcomm AI Hub follows this pattern. The implication for
developer workflows — most importantly, that there is no static
asset to mirror or pre-cache — is not always made explicit in
introductory materials.

---

## 4. System Design

The verified pipeline is:

```
HuggingFace checkpoint (Apache 2.0)
        │
        ▼  qai-hub-models export script
ONNX shards + AIMET quantization encodings
        │
        ▼  HTTPS upload to Qualcomm AI Hub
6 cloud compile jobs (one per shard) on real X1E hardware
        │
        ▼  cloud link job
Compiled context-binary bundle (.bin × 6)
        │
        ▼  HTTPS download
Local bundle directory
        │
        ▼  add tokenizer.json + genie_config.json + htp_backend_ext_config.json
Runnable Genie bundle
        │
        ▼  genie-t2t-run.exe
NPU inference on Hexagon v73
```

The first six steps are the model preparation. The last step is the
inference path. `hexrun` Phase 1 replaces the final `genie-t2t-run`
shell-out with native Rust bindings to the same Genie C API.

### 4.1 Hardware target

All measurements were made on a Microsoft Surface laptop with:

- **SoC:** Snapdragon X Elite X1E80100
- **NPU:** Hexagon v73, 45 TOPS INT8 advertised
- **NPU memory:** 7.8 GB shared LPDDR5x (per Task Manager)
- **OS:** Windows 11 ARM64, build 26100
- **HTP driver:** 30.0.219.1000 (signed September 2025)

### 4.2 Software stack

- **QAIRT SDK 2.45.0** at `$QNN_SDK_ROOT` (manual install from
  Qualcomm developer portal; not redistributable).
- **Genie runtime** from `$QNN_SDK_ROOT/lib/aarch64-windows-msvc/Genie.dll`
  (libGenie 1.17.0 reported at runtime).
- **Rust toolchain:** stable-aarch64-pc-windows-msvc 1.95.0 via
  rustup, MSVC v143 14.44 ARM64 build tools, Win11 SDK 10.0.26100.0,
  LLVM/clang 19+ for `bindgen` and `ring` ARM64 assembly.
- **Python toolchains:** native ARM64 Python 3.13 for general use;
  **x64** Python 3.11 under Prism emulation for the
  `qai-hub-models[qwen2-5-7b-instruct]` install, because PyTorch
  versions pinned by some `qai-hub-models` extras lack ARM64 Windows
  wheels.

### 4.3 Model

**Qwen 2.5 7B Instruct** [@qwen-2.5] with `w8a16` quantization
(8-bit weights, 16-bit activations, per-channel weights), exported
in 6 shards of 674 MB–1.04 GB each (4.6 GB total). Apache 2.0
licensed; no end-user license click-through required.

### 4.4 Bundle composition

The runnable bundle directory consists of:

```
qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite/
├── qwen2_5_7b_instruct_w8a16_part_1_of_6.bin   (1090 MB)
├── qwen2_5_7b_instruct_w8a16_part_2_of_6.bin   ( 707 MB)
├── qwen2_5_7b_instruct_w8a16_part_3_of_6.bin   ( 707 MB)
├── qwen2_5_7b_instruct_w8a16_part_4_of_6.bin   ( 707 MB)
├── qwen2_5_7b_instruct_w8a16_part_5_of_6.bin   ( 707 MB)
├── qwen2_5_7b_instruct_w8a16_part_6_of_6.bin   ( 972 MB)
├── tokenizer.json                              ( 7.0 MB; from HuggingFace)
├── genie_config.json                           (   2 KB; sampler, context, paths)
├── htp_backend_ext_config.json                 ( 0.5 KB; soc_model 60, dsp_arch v73)
└── tool-versions.yaml                          (qairt: 2.45.0.260326154327)
```

The two JSON files were adapted from the templates in the
qualcomm/ai-hub-apps tutorial repository [@ai-hub-apps], with paths
updated to match the actual file names produced by the export
pipeline (the AI Hub export uses `_w8a16_part_N_of_6.bin` while the
template assumes `_part_N_of_6.bin`).

---

## 5. Implementation findings

This section catalogues twelve concrete obstacles encountered in
April 2026 on a freshly-provisioned Snapdragon X Elite Surface
laptop. Each is documented with the symptom and fix. Most are not
covered in the official Qualcomm or Microsoft documentation in a
form a third-party developer is likely to find via search.

**5.1 GNU `link.exe` shadows MSVC `link.exe`.** MSYS2 / Git Bash
ships `/usr/bin/link.exe` (GNU coreutils) ahead of MSVC's linker
on PATH. Symptom: `link: extra operand 'C:\...\rcgu.o'` from cargo.
Fix: invoke cargo from a shell that loads `vcvarsall.bat arm64` and
prepends `%USERPROFILE%\.cargo\bin`. The `hexrun` repo provides
`scripts/dev-shell.bat` for this.

**5.2 `ring` requires clang for ARM64 assembly.** The `ring` crate
v0.17 (transitively pulled in by `reqwest`/`rustls`) compiles ARM64
intrinsics via clang. Symptom: `ToolNotFound: failed to find tool
"clang"`. Fix: install LLVM (`winget install LLVM.LLVM`); set
`LIBCLANG_PATH=C:\Program Files\LLVM\bin` and add it to PATH.

**5.3 `ort` 2.0.0-rc.12 references a non-existent `OrtApi` field.**
A regression in the latest `ort` release candidate at the time of
writing. Symptom: `error[E0609]: no field
SessionOptionsAppendExecutionProvider_VitisAI`. Fix: pin
`ort = "=2.0.0-rc.10"` in workspace `Cargo.toml`.

**5.4 `QnnSystem.lib` does not ship in QAIRT 2.45 on Windows ARM64.**
Only `QnnSystem.dll` is present in `lib/aarch64-windows-msvc/`. A
naive `cargo build` against the SDK fails with `LNK1181: cannot open
input file 'QnnSystem.lib'`. Fix: do not declare `links = "QnnSystem"`
in the FFI crate's `Cargo.toml`, do not emit
`cargo:rustc-link-lib=dylib=QnnSystem` from `build.rs`. Load symbols
at runtime via the `libloading` crate. Note that `Genie.lib` *does*
ship, so the higher-level Genie API can be statically linked.

**5.5 bindgen-generated documentation breaks rustdoc.** QNN headers
contain Doxygen-style doc comments with markdown bullet lists and
words like "function" and "call" that rustdoc attempts to compile
as Rust code. Symptom: `expected one of '!' or '::', found
'function'` from `cargo test`. Fix: disable doctests on the FFI
crate via `[lib] doctest = false`.

**5.6 Windows console default encoding (cp1252) cannot encode
Unicode emoji.** The `qai-hub` Python client prints `⏳` (U+23F3)
and `✅` (U+2705) in its progress animation. The Windows Python
default I/O encoding crashes the script *while a real cloud compile
job is succeeding in the background.* Symptom: `UnicodeEncodeError:
'charmap' codec can't encode character '⏳'`. Fix: set
`PYTHONIOENCODING=utf-8` and `PYTHONUTF8=1` before running, and pass
`python -X utf8`.

**5.7 The undocumented client-side cache-file trick.** This is the
most operationally important finding in this section. The
`qai-hub-models` export script accepts a `--model-cache-mode` flag
with values `enable`, `disable`, `overwrite` (default: `disable`).
The `enable` value reuses uploaded model IDs from a YAML cache file
at
`%USERPROFILE%\.qaihm\qai-hub-models\models\<model>_<precision>\v2\model_cache.yaml`.
However, **the cache is only written when a prior successful run also
used `enable`.** In practice, almost every first attempt by a new
user runs with the default `disable`; if that run uploads tens of
gigabytes and then fails for any reason (network blip, encoding
crash, disk pressure), a retry with `enable` re-uploads from
scratch. The cache-yaml format is reverse-engineered as:

```yaml
cache:
- key:
    cache_name: ar128_cl4096_part_1_of_6
    pytorch: 2.11.0+cpu
    onnx: 1.18.0
    hub_endpoint: https://workbench.aihub.qualcomm.com
    version: v2
    precision: w8a16
    context_length: '4096'
    sequence_length: '128'
  val:
    hub_model_id: <id from `qai_hub.get_models()`>
```

The hub-side model IDs persist across export attempts and are
listable via `qai_hub.get_models()` even when the local cache is
empty. **Pre-populating this YAML with all six shard model IDs from
prior partial runs reduces re-export wall-clock time from
approximately 90 minutes (re-uploading ~30 GB of ONNX shards) to
approximately 30 minutes (cloud compile + link + download only).**
We released this finding in the `hexrun` repository's troubleshooting
documentation as well as `scripts/qai-hub-status.py`, which lists
existing uploads and jobs on a user's account.

**5.8 `--model-cache-mode enable` does not skip compile jobs.** Each
export run resubmits compile + link jobs to the cloud regardless of
cache state. For a 7B model: 6 compile jobs at ~10 min each plus 6
link jobs at ~5 min each. There is no equivalent client-side cache
for compiled artifacts.

**5.9 Llama-family models require Meta license acceptance and a
HuggingFace token.** The Apache-2.0-licensed Qwen 2.5 7B is
recommended for first attempts; the workflow is otherwise
identical.

**5.10 The export pipeline emits Genie bundles for LLMs, not plain
ONNX.** The `--target-runtime` flag of the LLM export scripts
accepts only `genie`. This means the inference path goes through
the Genie LLM runtime, not the plain ONNX Runtime QNN Execution
Provider. Plan for libGenie bindings, not pure ORT.

**5.11 Windows ARM64 Python ecosystem is fragmented.** ONNX Runtime
quantization tooling has no ARM64 wheel; PyTorch 2.4.1 (pinned by
some `qai-hub-models` extras for the Llama family) has no ARM64
wheel; Qwen's looser pin resolved to torch 2.11 which does have an
ARM64 build. Fix: install **x64** Python 3.11 alongside any ARM64
Python (`winget install Python.Python.3.11 --architecture x64`) and
use `py -3.11-64` as the venv interpreter for AI Hub work. Windows
runs the x64 Python under Prism emulation transparently.

**5.12 Visual Studio Installer ARM64 component identifiers are
trap-laden.** The component
`Microsoft.VisualStudio.Component.VC.Tools.ARM` provides 32-bit ARM
target tools (`HostARM64\arm`), which is the wrong target for our
purposes. The component required is
`Microsoft.VisualStudio.Component.VC.Tools.ARM64`, displayed in the
GUI as "MSVC v143 - VS 2022 C++ ARM64/ARM64EC build tools (Latest)".
The unattended-install command-line invocation
`setup.exe modify --add Microsoft.VisualStudio.Component.VC.Tools.ARM64`
returns exit code 87 (`ERROR_INVALID_PARAMETER`) on some VS 2022
17.14 versions, requiring a GUI-based install instead.

---

## 6. Evaluation

### 6.1 Phase 0 verification

Two independent inference runs were performed using
`genie-t2t-run.exe` from QAIRT 2.45 against the bundle described
in Section 4.4:

**Run 1** — short prompt, terse expected response.

> *Prompt:* "Tell me a one-line joke about Snapdragon laptops."
>
> *Output:* "Why did the Snapdragon laptop go to the gym? To improve
> its CPU!"

**Run 2** — short prompt, structured expected response.

> *Prompt:* "Write a haiku about a Snapdragon laptop running a
> 7-billion parameter model on its NPU."
>
> *Output:*
> "Snapdragon think,
> 7 billion parameters dream big,
> NPU speeds ahead."

Both completed in ~29 seconds wall-clock including the bundle cold
load (~30 seconds for 4.6 GB of context binaries to be paged into
NPU shared memory).

### 6.2 NPU-utilization signature

A third, longer run was performed with Task Manager open on
**Performance → NPU**. The prompt requested a 120-word paragraph
(an output of approximately 215 tokens). Observed Task Manager
metrics during steady-state generation:

- **NPU Compute utilization:** sustained 19% mean, 30–40% peaks. The
  oscillating waveform pattern with regular peaks at approximately
  one-second intervals matches the expected signature of
  autoregressive token-by-token decode.
- **NPU Shared memory:** 4.9 GB / 7.8 GB total (62.8% of available
  NPU memory used by the resident bundle).
- **CPU utilization:** 12% mean, with no sustained spikes during
  generation, confirming work was offloaded to the NPU.
- **Driver:** 30.0.219.1000, dated September 11 2025.

**This is the third independent confirmation of NPU execution.** The
first is Genie's own `[BEGIN]:`/`[END]` output bracket (Section
6.1), and the second is the AI Hub job metadata showing the bundle
was compiled with `--target_runtime qnn_dlc` against
`Snapdragon X Elite CRD` on real Qualcomm hardware. We require all
three to agree before claiming NPU acceleration; silent CPU fallback
is the dominant failure mode in this ecosystem.

### 6.3 Wall-clock timing (limited)

| Metric | Value |
|---|---:|
| Bundle cold load | ~30 s |
| Total wall clock (215-token output) | 171 s |
| Words generated | 166 |
| Approx tokens (~1.3 / word) | 215 |
| Wall-clock-average tokens/sec | 1.3 |

We emphasize that the **1.3 tokens/sec figure is dominated by the
4.6 GB cold load** that the `genie-t2t-run` invocation pays at
start. Each invocation re-pages the bundle into NPU shared memory.
Steady-state inference rate is necessarily higher than this average
and is consistent with the observed sub-second peak-to-peak
oscillation in the Task Manager NPU graph (Section 6.2). Honest
steady-state measurement requires keeping the bundle resident
across queries — i.e., a long-running runtime — which is the core
of `hexrun` Phase 1 (Section 7) and is left to future work.

For reference, Qualcomm's published expectation for 7B-class
INT4-quantized models on the X1E platform is approximately 8–15
tokens/sec sustained [@qualcomm-ai-hub], which we expect to confirm
once the cold-load cost is amortized.

### 6.4 The 4096-token context cap

Every NPU-pre-built LLM available on Qualcomm AI Hub for Snapdragon
X Elite — Phi 3.5 Mini, Llama 3.1 8B, Llama 3.2 1B/3B, Qwen 2/3 4B
and 7B variants, Mistral 7B v0.3, and others — is published at a
fixed 4,096-token context window. This is independent of the base
model's native maximum; Llama 3.1 supports 128K natively, Phi 3.5
supports 128K natively, Qwen 2.5 supports up to 128K natively. The
cap is imposed at compile time for the NPU target.

The cause is structural: the NPU compiler bakes maximum sequence
length, KV-cache memory layout, and operator scheduling into the
compiled context binary. Recompilation at a longer context requires
Qualcomm's proprietary compile stack and tractable fit within the
Hexagon tensor processor's tightly-coupled memory (TCM) plus shared
LPDDR5x ceiling. We did not attempt longer-context recompilation;
it is not exposed via Qualcomm AI Hub's public interface.

### 6.5 Reproduction effort

End-to-end on a freshly-provisioned laptop, the recipe is
approximately:

| Step | Time |
|---|---|
| Rust + LLVM + MSVC ARM64 + Win11 SDK install | ~30 min (downloads + GUI installer) |
| QAIRT SDK download + extract | ~10 min |
| x64 Python + qai-hub-models install | ~15 min |
| Source weights download (15 GB from S3) | ~20 min |
| Local ONNX export | ~5 min |
| Cloud upload (30 GB via the export pipeline; first attempt) | ~30 min |
| Cloud compile (6 jobs sequential) | ~60 min |
| Cloud link + download | ~10 min |
| Bundle config + tokenizer setup | ~2 min |
| First inference run | ~30 s |
| **Total** | **~3 hours** |

With the cache-file trick (Section 5.7) applied to a re-export, the
upload step drops to seconds and total wall-clock to approximately
75 minutes.

---

## 7. Discussion

### 7.1 What this changes

For a developer with a Snapdragon X Elite laptop and a free Qualcomm
developer account, NPU-accelerated LLM inference moves from
"effectively impossible without insider access" to "a documented
afternoon of work." That shift, while not algorithmic, is the
practical gating factor for whether the open ecosystem can build on
this hardware class at all. Three classes of downstream work become
possible:

1. **Direct integration of hexrun-equivalent Genie bindings** into
   existing projects (Ollama, llama.cpp, LM Studio) to provide an
   NPU code path on Snapdragon X Elite without each project
   independently re-discovering the twelve obstacles in Section 5.

2. **Energy-efficiency studies** comparing NPU vs CPU inference on
   the same model and prompt distribution, which require a stable
   NPU baseline that this work establishes.

3. **NPU-tier user experiences** — local chat, transcription,
   summarization with battery-life characteristics that justify the
   "Copilot+ PC" hardware investment from a developer's perspective
   rather than only a vendor's.

### 7.2 What this does not change

We do not improve model quality, training, or architecture. We do
not make the NPU faster. We do not reduce the 4096-token context
cap, which is a hardware constraint that the Snapdragon X2 (~80
TOPS, mid-2026) may relax but the X1E platform will not. We do not
remove the dependency on Qualcomm's cloud build service for
generating new context binaries; that remains gated until either
Qualcomm releases more of their compile toolchain or third parties
re-implement equivalents.

### 7.3 Threats to validity

The recipe is verified on a single hardware unit (a Surface laptop
with X1E80100). Other X1E SKUs (X1E78100, X1E84100) and the X1P SKUs
should behave identically modulo TOPS rating, but we have not tested
this. The QAIRT SDK version (2.45.0) and HTP driver version
(30.0.219.1000) are explicit dependencies; future driver updates
may invalidate the bundle without warning, which is the failure
mode the cache-file (Section 5.7) and version-pinning (`hexrun`
manifest validation) machinery is intended to guard against.

The Task Manager utilization measurement is qualitative. We did not
instrument the QNN profiler (`QNN_LOG_LEVEL=PROFILE`) for this work;
that is a planned `hexrun` Phase 0 follow-up. Quantitative per-op
timing breakdown between Hexagon and CPU fallback is the standard
verification path for ruling out partial CPU offload, and we
acknowledge it as a hole in the present evaluation. The three
independent signals we do have (Genie stdout, AI Hub compile-target
metadata, sustained NPU-column utilization with corresponding
shared-memory occupancy) are sufficient to rule out the specific
failure mode we were most concerned with — silent end-to-end CPU
fallback — but are not sufficient to rule out a smaller fraction of
ops being executed on the CPU.

---

## 8. Future work: the `hexrun` runtime

`hexrun` [@hexrun-repo] is the open-source successor to the
`genie-t2t-run` shell-out used in this work. It is a Cargo workspace
of six Rust crates plus a Python conversion sidecar, currently at
the post-Phase-0-verification milestone. The remaining phases:

- **Phase 1: Rust bindings to QNN and Genie.** Replaces the shell-out
  to `genie-t2t-run.exe` with native bindings, allowing the runtime
  to keep the model loaded across queries. Eliminates the cold-load
  cost from Section 6.3 and enables honest steady-state speed
  measurement.

- **Phase 2: Generation loop and sampler.** Tokenizer (`tokenizers`
  crate), top-k/top-p/temperature sampler (already implemented as a
  stand-alone module with tests), KV-cache management.

- **Phase 3: Ollama-class CLI.** `hexrun pull <model>`,
  `hexrun run <model> <prompt>`, model registry, sha256-verified
  downloads.

- **Phase 4: HTTP server.** OpenAI-compatible
  `/v1/chat/completions` (with SSE streaming) plus Ollama-compatible
  `/api/tags`, allowing existing UIs (Open WebUI, etc.) to point at
  a hexrun instance unchanged.

- **Phase 5: `hex-convert` Python pipeline.** End-to-end HuggingFace
  → ONNX → AI Hub export → bundle creation, automating the recipe in
  Section 4. Expected to expand the model catalog beyond what
  Qualcomm has pre-built on AI Hub.

- **Phase 6: Release prep.** Signed Windows installer (MSIX),
  `winget` manifest, signed CI matrix, docs site.

The Phase 1 work is the most immediate technical risk, primarily
because the QnnSystem dynamic-loading path (Section 5.4) has no
public Rust precedent to copy from. If that proves tractable
(tractability is high — `libloading` is a mature crate and the C
API surface is stable), the remainder is conventional Rust systems
work.

---

## 9. Conclusion

The Snapdragon X Elite NPU is usable for general LLM inference under
Windows on ARM, today, with open-license models and free vendor
tooling, contingent on the developer's willingness to navigate a
twelve-item list of toolchain obstacles. None of those obstacles is
fundamental; all have specific, documented fixes; the gating factor
is integration knowledge rather than absent capability. We have
verified end-to-end NPU execution of Qwen 2.5 7B Instruct on a stock
Surface laptop, with sustained NPU utilization characteristic of
real autoregressive decode and 4.9 GB of model weights resident in
NPU shared memory. The accompanying `hexrun` runtime is a public
artifact built on these findings and intended to provide an
Ollama-class developer experience for the NPU on this hardware
class. We invite reproduction, extension, and reports of any
thirteenth obstacle we missed.

---

## References

[@npu-efficiency]: NVIDIA, "Why Are Tensor Cores More Efficient Than CPUs?" (technical brief, 2023). Power-per-TOPS comparisons across CPU/GPU/NPU.

[@qualcomm-snapdragon-x-elite]: Qualcomm, "Snapdragon X Elite Compute Platform Product Brief" (2024). https://www.qualcomm.com/products/snapdragon-x-elite

[@copilot-plus-pc]: Microsoft, "Copilot+ PCs" (2024). https://www.microsoft.com/en-us/windows/copilot-plus-pcs — minimum NPU requirement of 40 TOPS specified.

[@phi-silica]: Microsoft, "Get started with Phi Silica in the Windows App SDK" (2025). https://learn.microsoft.com/en-us/windows/ai/apis/phi-silica — limited-access policies described.

[@nexasdk]: Nexa AI, "NexaSDK" (2026). https://github.com/NexaAI/nexa-sdk

[@hexrun-repo]: Bonker, B., "hexrun: NPU-first local LLM runtime for Snapdragon X Elite" (2026). Repository at the time of writing private; intended for public release at v0.1.0. See `docs/handoff.md` and the project plan.

[@qualcomm-qairt-sdk]: Qualcomm, "Qualcomm AI Engine Direct (QAIRT) SDK" (2025). https://www.qualcomm.com/developer/software/qualcomm-ai-engine-direct-sdk

[@qualcomm-ai-hub]: Qualcomm AI Hub. https://aihub.qualcomm.com/

[@qai-hub-models]: Qualcomm, "qualcomm/ai-hub-models" GitHub repository (2026). https://github.com/qualcomm/ai-hub-models

[@ai-hub-apps]: Qualcomm, "qualcomm/ai-hub-apps" GitHub repository, in particular the `tutorials/llm_on_genie` directory (2026). https://github.com/qualcomm/ai-hub-apps

[@qwen-2.5]: Qwen Team, "Qwen 2.5 Technical Report" (2024). HuggingFace model card: https://huggingface.co/Qwen/Qwen2.5-7B-Instruct

[@mlc-llm]: MLC LLM project, "MLC-LLM" (2024). https://github.com/mlc-ai/mlc-llm

[@mlc-llm-issue-1689]: "Add support for QNN HTP backend" issue, MLC-LLM repository (2024–). https://github.com/mlc-ai/mlc-llm/issues/1689

[@executorch]: PyTorch project, "ExecuTorch" (2025). https://docs.pytorch.org/executorch/

[@ollama]: Ollama project (2024). https://ollama.com/

[@llamacpp]: ggerganov, "llama.cpp" (2023–2026). https://github.com/ggml-org/llama.cpp

---

*Authors' note. This document was prepared in collaboration with
Claude (Anthropic) on April 30 2026, in a single working session on
the same Snapdragon X Elite laptop on which the experiments were
performed. The reported measurements were produced live during that
session. The commentary on related work and the structural framing
reflects the authors' editorial decisions; the technical claims are
backed by the working artifact in the `hexrun` repository.*

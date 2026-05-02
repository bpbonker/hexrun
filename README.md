# npurun

**NPU-first local LLM runtime for Snapdragon X-series Windows-on-ARM laptops.**

The 45 TOPS Hexagon NPU on Snapdragon X laptops is real and chat-pace
usable. The open ecosystem just doesn't reach it: Ollama, llama.cpp,
LM Studio, and text-generation-webui all run CPU-only on these chips.
Other runtimes *do* hit the NPU — NexaSDK and AnythingLLM's bundled
QNN engine both wrap the same underlying Qualcomm Genie SDK, and
Microsoft's Phi Silica targets it from first-party Copilot apps —
but each is closed-source, gated, or SoC-string-pinned in ways that
make them awkward to embed or extend.

**npurun is the open Rust path.** Same Genie SDK that Qualcomm's own
tooling uses, exposed through an Ollama-class CLI (`pull` / `run` /
`serve` / `bench` / `ps`) and an OpenAI- and Ollama-compatible HTTP
server. Multi-model registry with sha256-verified pulls, multi-turn
chat via Genie's KV-cache rewind, automatic multi-graph bundle
support (the `enable-graph-switching` injection that turns 0.6 tok/s
into 14.9 tok/s on Qwen3-4B). Verified on hardware.

> **Status:** working preview; tagged builds incoming. Qwen3-4B
> Instruct 2507 hits **~14.9 tok/s** under `npurun bench` on the X1E
> NPU; Phi 3.5 Mini ~11.7 tok/s; Qwen 2.5 VL-7B (w4a16) ~9.1 tok/s
> text-only. See [`docs/benchmarks.md`](docs/benchmarks.md) for the
> raw runs and methodology.

---

## Quick start (≈5 minutes once prerequisites are in place)

```powershell
# Install npurun once your prerequisites are in place (see below).
cargo install --path crates/npurun-cli

# Download a model — auto-extracts and writes a manifest.
npurun pull qwen3-4b-instruct-2507

# Run a one-shot generation. Streams tokens to stdout.
npurun run qwen3-4b-instruct-2507 "Tell me a one-line joke about Snapdragon laptops."

# Or run as an OpenAI/Ollama-compatible HTTP server.
npurun serve --model qwen3-4b-instruct-2507

# Then point Open WebUI (or any OpenAI/Ollama client) at:
#   http://localhost:11435
```

That's it. Three commands take you from a fresh laptop with prerequisites
to NPU-accelerated chat. See [`docs/handoff.md`](docs/handoff.md) for the
full operational state.

## What's in the box

| Subcommand | What it does |
|---|---|
| `npurun pull <name>` | Download a known model, extract, auto-write `npurun.json`. sha256 verified. Resumable. |
| `npurun list` | Show locally cached models. |
| `npurun show <name>` | Print the manifest of a cached model. |
| `npurun run <name> "<prompt>"` | One-shot generation; streams to stdout. |
| `npurun bench <name>` | Warm-query benchmark; per-prompt + aggregate tokens/sec. |
| `npurun serve --model <name>` | OpenAI- and Ollama-compatible HTTP server. SSE streaming, CORS, optional bearer-token auth. |
| `npurun rm <name>` | Delete a cached model. |
| `npurun ps` | Probe a running `npurun serve` and print model + uptime + auth state. |
| `npurun version` | Print npurun, libGenie, and QAIRT SDK versions. |

## Where npurun fits

| Tool | NPU on Snapdragon X-series (May 2026) | License | Notes |
|---|---|---|---|
| Ollama | CPU only ([#5360](https://github.com/ollama/ollama/issues/5360)) | open | Default for most users; misses the NPU. |
| llama.cpp | QNN backend stalled ([#8273](https://github.com/ggml-org/llama.cpp/discussions/8273)) | open | GGUF can't currently target the Hexagon NPU. |
| LM Studio | CPU/GPU only ([#30](https://github.com/lmstudio-ai/lms/issues/30)) | closed | — |
| text-generation-webui | none ([#6298](https://github.com/oobabooga/text-generation-webui/issues/6298)) | open | — |
| Microsoft Phi Silica | NPU | closed (Windows AI APIs) | Locked to first-party Copilot+ apps; specific Microsoft-shipped models. |
| NexaSDK | NPU (Genie under the hood) | closed CLI | Working NPU runtime, but not embeddable / inspectable. |
| AnythingLLM bundled QNN engine | NPU (Genie under the hood) | open frontend, closed engine | Works on X Elite; broken on X Plus / X 10-core where it pins on SoC marketing strings ([#2962](https://github.com/Mintplex-Labs/anything-llm/issues/2962), [#5129](https://github.com/Mintplex-Labs/anything-llm/issues/5129)). |
| **npurun** | NPU (Genie, native Rust FFI) | MIT / Apache-2.0 | Open Rust crate, embeddable as a library; doesn't gate on SoC strings (probes `Win32_VideoController` for the Hexagon NPU); ships an OpenAI- and Ollama-compatible HTTP server; multi-graph bundle support; multi-turn chat via Genie KV-cache rewind. |

What npurun is **not**: a faster Genie. Per-token decode speed comes
from libGenie itself — the same underlying runtime NexaSDK and
AnythingLLM use. What npurun adds is the open, embeddable, scriptable
surface around it: a Rust crate (`npurun-core`) you can pull into your
own project, an Ollama-class CLI for ad-hoc use, an
OpenAI/Ollama-shaped HTTP server for any client that already speaks
those APIs, and a registry/pull pipeline so users don't have to
hand-assemble Genie context bundles.

The table also omits CPU/GPU paths because that's not the gap. Most
of these tools run fine on the Snapdragon X CPU — llama.cpp also runs
on the Adreno GPU via Vulkan/OpenCL — and that's the right pick if you
want raw GGUF flexibility. The 45 TOPS Hexagon NPU is the part that
sits idle on those paths. npurun is the open path that uses it.

## Performance on real hardware

| Model | Hardware | Steady-state | TTFT |
|---|---|---:|---:|
| Qwen3-4B Instruct 2507 (w4a16, NPU) | X1E | **~14.9 tok/s** | ~120 ms |
| Phi 3.5 Mini (w4a16, NPU) | X1E | **~11.7 tok/s** | **~200 ms** |
| Qwen 2.5 VL-7B Instruct (w4a16, NPU, text-only) | X1E | **~9.1 tok/s** | ~156 ms |
| llama.cpp on the same laptop's CPU (Phi 3.5 Q4) | X1E CPU | ~5–8 tok/s (estimated) | — |

w4a16 multi-graph bundles in the 4B–7B range run at chat-pace on the
NPU and beat CPU paths on the same laptop. **~1.27 J/token at ~6.9 W
delta** on Phi 3.5 Mini, measured on battery — roughly 2–3× more
energy-efficient than the CPU path
(see [`docs/benchmarks.md`](docs/benchmarks.md) for methodology).

## Prerequisites

| Requirement | Why | How |
|---|---|---|
| Snapdragon X-series laptop (X Elite, X Plus, or X2 — anything with a Hexagon NPU and QAIRT support) | Hexagon NPU is the whole point | — |
| Windows 11 24H2+ on ARM64 | Required for current HTP driver and QAIRT 2.44+ | — |
| Rust stable + `aarch64-pc-windows-msvc` target | builds the runtime | `winget install Rustlang.Rustup` then `rustup target add aarch64-pc-windows-msvc` |
| MSVC v143 ARM64/ARM64EC build tools + Win11 SDK 26100 | linker/toolchain | Visual Studio Installer → Modify VS 2022 → Individual Components |
| LLVM/clang | bindgen + `ring` ARM64 asm | `winget install LLVM.LLVM` |
| QAIRT SDK 2.44+ | NPU runtime + Genie | manual download from [Qualcomm developer portal](https://www.qualcomm.com/developer); set `QNN_SDK_ROOT` to install path |

The QAIRT SDK is **not redistributable** — install it manually from the
Qualcomm developer portal. Run `pwsh -File scripts\setup-qnn.ps1` to
validate the install before building npurun.

For full setup details and every toolchain papercut we hit while
building this, see [`docs/troubleshooting.md`](docs/troubleshooting.md)
and [`docs/handoff.md`](docs/handoff.md).

## Built-in model registry

`npurun pull <name>` knows about a small set of pre-built bundles
hosted on Qualcomm's HuggingFace org:

| Name | Size | Verified |
|---|---:|---|
| `qwen3-4b-instruct-2507` | ~2.5 GB | chat-usable, **~14.9 tok/s** (`npurun bench`) — current NPU ceiling |
| `qwen3-4b` | ~2.5 GB | base model, same multi-graph format as Instruct-2507 |
| `phi-3.5-mini` | ~2.1 GB | chat-usable, 11.7 tok/s |
| `qwen-2-5-vl-7b-instruct` | ~4.9 GB | 7B vision-language, **9.1 tok/s** text-only (vision pipeline present, not exercised by npurun yet) |
| `llama-v3-1-8b-instruct` | ~4.5 GB | not precompiled by Qualcomm — self-compile only |

> **Multi-graph bundles need a config flag.** Bundles published from
> late 2025 onwards (Qwen3, etc.) ship with `prompt_ar128_*` /
> `token_ar1_*` graph names. libGenie 1.17.0's auto-switch heuristic
> doesn't recognise this naming, so without `enable-graph-switching:
> true` in `genie_config.json` the runtime executes the prefill graph
> for every decode token and throughput collapses by ~20×. `npurun
> pull` injects this flag automatically after extraction. (Qwen 2.5
> VL-7B is structurally different and doesn't need the flag — it
> still gets injected today and costs ~400 ms TTFT on that bundle;
> follow-up to make injection conditional.)

A remote registry (signed JSON index) is on the roadmap. For adding
models that aren't in the built-in registry today, see
[`python/npu-convert/`](python/npu-convert/) — a Python sidecar that
takes any Genie bundle (downloaded or built locally) and writes the
`npurun.json` manifest the runtime needs.

## Architecture (one paragraph)

`qnn-sys` (raw FFI to QNN + Genie) ← `qnn` (safe Rust wrappers) ←
`npurun-core::Engine` (loads bundles, applies chat templates, runs
inference) ← `npurun-cli` (CLI) and `npurun-server` (axum HTTP server).
The server holds a single `Arc<Engine>` plus a `tokio::sync::Semaphore`
with a single permit so concurrent requests serialize cleanly with
HTTP 429s instead of head-of-line blocking. Streaming is async via
`mpsc` from a `spawn_blocking` Genie call. See
[`docs/architecture.md`](docs/architecture.md) for the design rationale.

## Verifying you're actually on the NPU

Three independent checks. **All three must agree** before claiming NPU
acceleration:

1. **Task Manager → Performance → NPU** shows sustained utilization
   during a `npurun run` — typically 19–30% for a 4 GB-class model with
   most of NPU shared memory in use.
2. The bundle's compile metadata says `target_runtime: qnn_dlc` against
   `Snapdragon X Elite CRD`. Check `npurun show <name>` for the manifest;
   the underlying compile happened in Qualcomm's cloud.
3. `npurun bench <name>` reports tokens/sec at least 3× a CPU baseline
   on the same model, **or** (more reliably) >5 tok/s for ~4B models on
   the NPU.

If the NPU column is at 0% but `npurun run` is producing text, you are
silently on CPU fallback — file an issue with the output of
`npurun version`.

## Server: LAN-deployable

`npurun serve` accepts `--bind 0.0.0.0:11435` so the server is reachable
from other devices on your network. Pair with `--auth-token <TOKEN>` to
require `Authorization: Bearer <TOKEN>` on `/v1/*` and `/api/*`.
Endpoints:

- OpenAI: `GET /v1/models`, `POST /v1/chat/completions` (blocking + SSE)
- Ollama: `GET /api/tags`, `GET /api/version`, `POST /api/generate`,
  `POST /api/chat` (blocking + NDJSON), `POST /api/show`,
  `POST /api/delete`
- Health: `GET /healthz` (returns JSON with model, uptime, version)

Ollama-style `<name>:latest` references work everywhere — the CLI, the
server, and `npurun ps` all strip the tag and serve the bare name.

CORS is permissive so browser-based clients (Open WebUI, custom UIs) can
hit the server cross-origin. Concurrent requests beyond one are rejected
with HTTP 429 + `Retry-After: 1` rather than queued indefinitely.

## Documentation

The full docs are also rendered as a [browsable site with search](https://bpbonker.github.io/npurun/).

- [`docs/handoff.md`](docs/handoff.md) — operational state and reproduction recipe
- [`docs/findings.md`](docs/findings.md) — engineering blog post / contribution writeup
- [`docs/benchmarks.md`](docs/benchmarks.md) — raw timings, methodology, comparison
- [`docs/paper.md`](docs/paper.md) — formal experience-report writeup
- [`docs/architecture.md`](docs/architecture.md) — design decisions
- [`docs/troubleshooting.md`](docs/troubleshooting.md) — every error we've hit and the fix
- [`docs/compatibility.md`](docs/compatibility.md) — model compatibility matrix
- [`docs/release.md`](docs/release.md) — copy-paste release runbook (build, tag, sign, winget)
- [`docs/roadmap.md`](docs/roadmap.md) — what's left for v0.1.0

## Status / roadmap (May 2026)

**Shipped:**

- [x] Phase 0 — NPU verified end-to-end with Qwen 2.5 7B on hardware
- [x] Phase 1 — Native Rust bindings to libGenie / QNN
- [x] Phase 2 — `npurun list/show/run` CLI
- [x] Phase 3 — `npurun pull/rm` with built-in registry
- [x] Phase 4 — HTTP server (OpenAI + Ollama compat, SSE/NDJSON streaming)
- [x] Server LAN safety: CORS, `--auth-token`, warmup, rich `/healthz`,
  HTTP 429 backpressure, graceful shutdown
- [x] Pull integrity: sha256 verification + resumable downloads
- [x] Ollama parity: `:latest` aliases, `/api/version`, `/api/show`,
  `/api/delete`, `npurun ps` against a running server
- [x] Energy measurement: ~1.27 J/token at ~6.9 W delta on Phi 3.5 Mini
- [x] Multi-turn chat via Genie KV-cache prefix matching
  (`SentenceCode::Rewind`) — turn N pays the prefill cost only for
  the new tokens, not the whole transcript

**In progress:**
- [x] Phase 5 (starter): `npu-convert manifest` + `npu-convert inspect`
  for local Genie bundles; `npu-convert export` shells out to
  `qai-hub-models`. Curated recipe set; remote registry still open.
- [x] Phase 6 (starter): winget manifest (zip installer, validated),
  tag-triggered release workflow, expanded CI matrix
  (pytest + winget-validate), dev-cert helper for signed-MSIX
  testing, full release runbook in [`docs/release.md`](docs/release.md).
- [ ] Phase 6 final: signed MSIX with a real CA / Azure Trusted
  Signing cert, public winget catalog submission, self-hosted ARM64
  CI runner enrolled.

See [`docs/roadmap.md`](docs/roadmap.md) for the detailed wave plan.

## Support the work

npurun is independently maintained. If it saved you time, or you want
to see the roadmap items above land sooner, you can [☕ buy me a coffee](https://buymeacoffee.com/bpbprofessional).

Tips fund the unglamorous parts — toolchain debugging, NPU SDK spelunking,
the third rewrite of an FFI binding nobody else has written.

## License

Dual-licensed under MIT or Apache-2.0 at your option.

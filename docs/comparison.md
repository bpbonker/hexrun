# Runtime comparison

How npurun stacks up against other ways to run the same models on the
same Snapdragon X-series laptop. Measured numbers are clearly marked;
cited numbers come with sources.

## Why a runtime comparison matters

The same hardware can run an LLM through several different software
stacks. Each picks a different tradeoff between throughput, energy, UX,
and openness. The numbers below are roughly what to expect; pick a
runtime based on which tradeoff matches your use case.

## Qwen3-4B Instruct 2507 (w4a16) on Snapdragon X Elite X1E80100

The current headline comparison: a 4B w4a16 multi-graph bundle.

| Runtime | Backend | Tok/s steady-state | TTFT | Source |
|---|---|---:|---:|---|
| **npurun** (`npurun bench`) | NPU (Genie) | **~14.9** | **~120 ms** | [`benchmarks.md`](benchmarks.md) — measured |
| AnythingLLM bundled QNN | NPU (Genie) | reported similar on supported SoCs | — | Same Genie runtime; broken on X Plus / X 10-core ([#2962](https://github.com/Mintplex-Labs/anything-llm/issues/2962), [#5129](https://github.com/Mintplex-Labs/anything-llm/issues/5129)) |
| NexaSDK | NPU (Genie) | reported similar | — | Closed runtime, can't independently verify |
| llama.cpp / Ollama | CPU (Oryon, 12-core) | ~6–10 (cited, smaller-context) | ~500 ms (cited) | Public benchmarks; CPU paths slower than NPU at this size |

## Phi 3.5 Mini (w4a16 / Q4) on Snapdragon X Elite X1E80100

The original headline comparison from Phase 1, kept because Phi has
the most-measured energy profile on this hardware.

| Runtime | Backend | Tok/s steady-state | TTFT | Power above idle | J/token | Source |
|---|---|---:|---:|---:|---:|---|
| **npurun** | NPU (Genie) | **~11.7** | **194 ms** | **~6.9 W** | **~1.27** | [`benchmarks.md`](benchmarks.md) — measured |
| AnythingLLM bundled QNN | NPU (Genie) | reported similar | — | — | — | Same Genie runtime; broken on X Plus / X 10-core ([#2962](https://github.com/Mintplex-Labs/anything-llm/issues/2962), [#5129](https://github.com/Mintplex-Labs/anything-llm/issues/5129)) |
| NexaSDK | NPU (Genie) | reported similar | — | — | — | Closed runtime, can't independently verify |
| llama.cpp | CPU (Oryon, 12-core) | ~5–8 (cited) | ~500 ms (cited) | ~12–18 W (cited) | ~2.0–3.5 (derived) | Public benchmarks; not yet measured on this machine |
| Ollama (default backend) | CPU (Oryon, 12-core) | ~5–8 (cited) | ~500 ms (cited) | ~12–18 W (cited) | ~2.0–3.5 (derived) | llama.cpp under the hood |

### Reading this table

- **Tok/s** is post-TTFT steady-state, not aggregate. Sampling
  back-to-back queries.
- **TTFT** is time-to-first-token from request submission, including
  prompt prefill but not bundle load.
- **Power above idle** is the additional watts pulled from the battery
  during inference, measured via `Win32_Battery.DischargeRate` on
  battery (display dim, no other apps).
- **J/token** is `(power × seconds) / tokens generated`. Lower is
  better; on battery, this directly determines how long a chat session
  costs.

## 7B-class on the NPU: Qwen 2.5 VL-7B (w4a16)

| Runtime | Backend | Tok/s steady-state | Notes |
|---|---|---:|---|
| **npurun** (`npurun bench`) | NPU (Genie) | **~9.1** | Text-only; vision pipeline present but not yet exercised by npurun. [`benchmarks.md`](benchmarks.md) — measured |
| llama.cpp generic 7B `Q4_K_M` | CPU (Oryon) | ~3–5 (cited) | Public benchmarks |

At 7B-class, the w4a16 multi-graph NPU path now beats the CPU path —
9.1 tok/s vs 3–5 tok/s. The earlier-generation Qwen 2.5 7B w8a16
bundle (1.9 tok/s) was the slower legacy path; see *Previous
iterations* in [`benchmarks.md`](benchmarks.md) for the historical
context.

## Picking a runtime

| Want | Pick |
|---|---|
| Best tok/s on a 4B-class model + lowest battery cost | **npurun** (Qwen3-4B Instruct 2507 on NPU, ~14.9 tok/s) |
| Best tok/s on a 7B-class model on this hardware | **npurun** (Qwen 2.5 VL-7B on NPU, ~9.1 tok/s text-only) |
| Easiest install, biggest GGUF model zoo | Ollama (CPU) |
| Any X-series silicon (Elite, Plus, X 10-core) | **npurun** — doesn't gate on SoC string |
| Workspace UI + RAG | AnythingLLM as a *client* against npurun (see [`integrations/anythingllm.md`](integrations/anythingllm.md)) |
| Microsoft Copilot+ apps | Phi Silica (closed, first-party only) |

## Adding your own measurement

If you have Ollama or llama.cpp installed, add a measured row by
running:

```powershell
# Ollama
ollama pull phi3.5
$env:OLLAMA_HOST = "http://127.0.0.1:11434"
Measure-Command { ollama run phi3.5 "Briefly explain why an NPU is more energy-efficient than a CPU." }

# llama.cpp
.\llama-bench.exe -m phi-3.5-mini-Q4_K_M.gguf -p 64 -n 256
```

Pair with `pwsh -File scripts\energy-bench.ps1` (adapted for the
runtime under test) for the joules-per-token side of the table. Open
a PR adding the row to this page; the goal is a public, reproducible
comparison anyone can verify on their own laptop.

## Caveats

- **Driver version matters a lot.** The HTP driver advances quickly;
  npurun's Phi 3.5 numbers are on driver 30.0.219.1000 (9/11/2025).
  llama.cpp's CPU numbers are CPU governor- and thermal-state-
  sensitive.
- **Plugged-in vs battery.** Most CPU runtimes throttle when on
  battery; the NPU does not (or does so much less). The energy
  story is most favourable for npurun on battery.
- **Single-user.** All these runtimes serialize a single inference
  request on a laptop. Don't extrapolate any of these numbers to
  shared / multi-tenant deployments.

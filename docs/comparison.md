# Runtime comparison

How npurun stacks up against other ways to run the same models on the
same Snapdragon X-series laptop. Measured numbers are clearly marked;
cited numbers come with sources.

## Why a runtime comparison matters

The same hardware can run an LLM through several different software
stacks. Each picks a different tradeoff between throughput, energy, UX,
and openness. The numbers below are roughly what to expect; pick a
runtime based on which tradeoff matches your use case.

## Phi 3.5 Mini (w4a16 / Q4) on Snapdragon X Elite X1E80100

| Runtime | Backend | Tok/s steady-state | TTFT | Power above idle | J/token | Source |
|---|---|---:|---:|---:|---:|---|
| **npurun** | NPU (Genie) | **~11.7** | **194 ms** | **~6.9 W** | **~1.27** | [`benchmarks.md`](benchmarks.md) — measured |
| AnythingLLM bundled QNN | NPU (Genie) | reported similar | — | — | — | Same Genie runtime under the hood; broken on X Plus / X 10-core ([#2962](https://github.com/Mintplex-Labs/anything-llm/issues/2962), [#5129](https://github.com/Mintplex-Labs/anything-llm/issues/5129)) |
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

## Qwen 2.5 7B (w8a16 / Q4_0_4_8)

| Runtime | Backend | Tok/s steady-state | Notes |
|---|---|---:|---|
| **npurun** (post `poll: true`) | NPU (Genie) | **~1.9** | [`benchmarks.md`](benchmarks.md) — measured |
| llama.cpp `Q4_0_4_8` | CPU (Oryon) | ~3–5 (cited) | [llama.cpp discussion #8273](https://github.com/ggerganov/llama.cpp/discussions/8273) — needs local confirmation |

For 7B-class models, the NPU is currently *slower* than CPU on this
generation of silicon. Use Phi 3.5 Mini (3.8B) on the NPU for
interactive chat; reach for llama.cpp / Ollama if you need a 7B
specifically.

## Picking a runtime

| Want | Pick |
|---|---|
| Best tok/s on a 4 GB-class model + lowest battery cost | **npurun** (NPU) |
| Best tok/s on a 7B-class model | llama.cpp / Ollama (CPU) |
| Easiest install, biggest model zoo | Ollama |
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

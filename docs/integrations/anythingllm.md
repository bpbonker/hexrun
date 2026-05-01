# AnythingLLM + npurun

[AnythingLLM](https://anythingllm.com/) is a popular local-first chat UI
with built-in document RAG, agents, and workspace management. It also
ships its own NPU inference path on Snapdragon X — but that path is
brittle: it string-matches the SoC marketing name and refuses to start
on anything other than a `Snapdragon(R) X Elite` 12-core (see issues
[#2962][allm-2962] and [#5129][allm-5129] — broken on X Plus and X
10-core variants).

The clean alternative is to let AnythingLLM keep doing what it's good
at — RAG, workspace UI, agents — and let `npurun` handle the NPU
inference behind an OpenAI-compatible endpoint. That bypasses
AnythingLLM's hardcoded SoC check completely; AnythingLLM never
touches Genie, never sees the SoC string, just talks HTTP to a port
on localhost.

## Setup

1. **Start `npurun serve`** with the model you want to use.

   ```powershell
   npurun serve --model phi-3.5-mini
   # listening on 127.0.0.1:11435
   ```

   Confirm with `npurun ps` in another shell.

2. **Open AnythingLLM → Settings → LLM Preference.**

3. **Pick "Generic OpenAI" as the provider.** Don't pick "Ollama" —
   AnythingLLM's Ollama client expects an `/api/version` payload
   shaped slightly differently than what most OpenAI-compat servers
   emit, and the Generic OpenAI path is more forgiving.

4. **Fill in:**

   | Field | Value |
   |---|---|
   | Base URL | `http://127.0.0.1:11435/v1` |
   | API Key | `dummy` (any non-empty string; `npurun serve` ignores it unless `--auth-token` is set) |
   | Chat Model Name | `phi-3.5-mini` |
   | Token context window | `4096` (or whichever tier you're targeting — see [`usage.md`](../usage.md) on `bench --ctx`) |
   | Max Tokens | `2048` is a safe default |

5. **Save and open a workspace.** First reply will pay the cold-load
   cost on `npurun serve` (~10s for Phi 3.5 Mini); every reply after
   that is warm.

If you set `--auth-token <TOKEN>` on `npurun serve`, paste the same
token into the API Key field instead of `dummy`.

## Why this works when AnythingLLM's own NPU mode doesn't

AnythingLLM's bundled QNN engine talks to libGenie directly and
gates startup on the SoC string. The Generic OpenAI provider doesn't
care what's behind the URL — could be OpenAI, could be Ollama, could
be `npurun serve`. As long as the server speaks
`POST /v1/chat/completions`, AnythingLLM will use it.

`npurun serve`, meanwhile, doesn't gate on the SoC string at all (see
`npurun show-hardware` for the probe it actually does). If libGenie
loads on your hardware, npurun runs.

## What you get

- AnythingLLM: workspace UI, document upload, vector store, citations,
  agent skills.
- npurun: NPU inference on Snapdragon X-series (Elite, Plus, X 10-core
  — anywhere libGenie loads), OpenAI- and Ollama-compatible endpoints,
  context-tier pinning, real `Genie::Status` errors instead of
  pre-flight rejections.

## Caveats

- **Single-user generation.** `npurun serve` serializes generation
  through one Genie permit (libGenie has no concurrency knob). If two
  AnythingLLM workspace tabs hit it at the same time, the second gets
  HTTP 429 and AnythingLLM surfaces it as an error in the chat. This
  is fine for individual use, not fine for shared deployments.
- **No embeddings yet.** AnythingLLM uses its own embedding model for
  RAG; that path stays on its native runtime. `npurun serve` does not
  expose `/v1/embeddings` — see [`roadmap.md`](../roadmap.md) for
  status.
- **Streaming works** but AnythingLLM occasionally buffers SSE chunks
  before rendering. That's a UI behaviour, not a server one.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| "Connection refused" | `npurun serve` isn't running, or AnythingLLM is in Docker without `host.docker.internal` mapped |
| "Model not found" | Chat Model Name doesn't match what `npurun ps` reports |
| Replies stop mid-sentence | Max Tokens set too low — bump to 2048+ |
| 429 errors | Another client is using the same `npurun serve` instance; wait and retry |

For deeper debugging, see [`troubleshooting.md`](../troubleshooting.md).

[allm-2962]: https://github.com/Mintplex-Labs/anything-llm/issues/2962
[allm-5129]: https://github.com/Mintplex-Labs/anything-llm/issues/5129

# Architecture

This document captures the load-bearing decisions behind npurun.

## Two backends, one engine

```
                     ┌──────────────────────┐
                     │     npurun-core      │
                     │    Engine::generate  │
                     │  (chat template,     │
                     │   tokenizer, KV       │
                     │   cache loop)         │
                     └──────────┬────────────┘
                                │
              ┌─────────────────┴─────────────────┐
              ▼                                   ▼
       Genie (default for LLMs)          ORT-QNN (fallback / non-LLM)
       ─────────────────────────         ───────────────────────────
       qnn::Dialog (Rust FFI to          ort::Session + QNN EP
       libGenie's C API)                 op dispatch + HTP cache
       loads pre-compiled qnn_dlc        scaffolded; not on the LLM
       (`qnn_ctx.bin`) bundles           hot path today
       from Qualcomm AI Hub
```

LLMs go through **Genie** because that is the format Qualcomm AI Hub
emits for chat-shaped models. The hub's LLM toolchain produces
multi-shard `qnn_dlc` bundles with `genie_config.json` + `tokenizer.json`
+ context binaries; libGenie owns the prefill/decode/sampler loop and
the KV cache. We wrap its C API directly via `qnn-sys` and `qnn`, so
cold start is fast and we don't pay for a re-implementation of pieces
the SDK already provides.

The **ORT-QNN path** stays in the workspace as the fallback for non-LLM
models (vision, embeddings, anything Qualcomm AI Hub exports as raw
ONNX). The `ort` crate plus the QNN Execution Provider give Op
coverage handled by Microsoft + Qualcomm and built-in HTP context-
binary caching. Today nothing on the LLM hot path uses it; it is
ready for the embedding endpoint and any future vision pipeline.

## Version pinning

The Nexa SDK #1060 trap was: ship a context binary built against QNN 2.36,
deploy onto a Windows update with a 2.40 driver — silent failures.

`qnn::Capabilities::probe()` snapshots SDK version + HTP driver version at
startup. Manifests embed the SDK version they were compiled against. On
load mismatch:

- patch (`2.44.0` vs `2.44.3`): silent.
- minor (`2.44.x` vs `2.45.x`): warn.
- major: refuse, ask user to re-pull.

When mismatch is detected and the source ONNX is present locally, we
auto-recompile the context binary in the background and surface progress
to the CLI.

## Why a single process

Ollama uses a daemon model — separate `ollama serve` + `ollama run`. Reasonable
choice but heavier on Windows (service registration, IPC). For v1, npurun is
a single binary; `npurun serve` just holds the engine in-process while the
HTTP server runs.

If we need a daemon later, the boundary is clean: `npurun-core::Engine` is
already `Send + Sync` and behind an `Arc`.

## Streaming

`tokio::sync::mpsc` from inference task → axum SSE response. Token-by-token,
no batching. Backpressure: if the HTTP client is slow, the channel fills
and the inference loop slows naturally.

## Why one permit instead of N

`npurun serve` holds an `Arc<Engine>` behind a `tokio::sync::Semaphore` with
a single permit. Concurrent requests beyond one return HTTP 429 with
`Retry-After: 1` rather than queuing. This is **not** a placeholder. It is
the load-bearing consequence of libGenie owning a single dialog handle whose
KV-cache state is mutated in place per query. Genie does not expose a
concurrency knob; running multiple inferences against one dialog corrupts
the cache. We already saw that failure mode in the bench dialog-reuse bug
fixed in commit `9611c7b`.

The sister project [specula](https://github.com/hotschmoe/specula) confirms
the same constraint. Their session-22 notes call out "NPU absent (Genie has
no concurrency knob)" and they had to spawn N processes to benchmark
ORT-QNN concurrency at all.

If we ever need real multi-tenant serving, the path is process fan-out
(each replica owns its own dialog) or an ORT-QNN backend with a
multi-session pool. Not adding permits to this semaphore.

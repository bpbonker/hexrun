# Integrations

`npurun serve` is an OpenAI- and Ollama-compatible HTTP server. Anything
that speaks either protocol works without modification. This section
collects short, tested recipes for the clients we know people are using.

## Quick start (any client)

Start the server once:

```powershell
npurun serve --model phi-3.5-mini
# listening on 127.0.0.1:11435
```

Point the client at one of:

| Protocol | Base URL |
|---|---|
| OpenAI | `http://127.0.0.1:11435/v1` |
| Ollama | `http://127.0.0.1:11435` |

If you set `--auth-token <TOKEN>`, supply it as the API key (or
`Authorization: Bearer <TOKEN>` header) in the client.

## Recipes

- [Bundled web chat (`/chat`)](web-chat.md) — open a browser at
  `http://127.0.0.1:11435/chat`. Zero install. Lowest-friction
  way to see npurun work.
- [curl + raw HTTP](curl.md) — the smallest possible smoke test.
- [Python (`openai` SDK)](python.md) — drop-in for any code that
  already targets OpenAI.
- [JavaScript / TypeScript (`openai` package)](javascript.md) — same
  story for Node and the browser.
- [Open WebUI](open-webui.md) — full chat UI in Docker.
- [AnythingLLM](anythingllm.md) — workspace UI + document RAG using
  the Generic OpenAI provider, bypassing AnythingLLM's broken bundled
  QNN engine.
- [Continue.dev / VS Code](continue.md) — code completion and chat
  inside the editor.
- [Ollama-flavoured clients](ollama-clients.md) — anything that reads
  `OLLAMA_HOST` (the Ollama CLI itself, lots of TUIs).

## Caveats that apply to every recipe

- **Single-user generation.** `npurun serve` serializes through one
  Genie permit; the second concurrent request gets HTTP 429. Fine for
  individual use, not fine for shared deployments.
- **Cold-load.** First request after `npurun serve` starts pays the
  9–11 s bundle load. The startup warmup query covers that for the
  first user; if you pass `--no-warmup`, the first client pays it.
- **No embeddings yet.** RAG-shaped clients fall back to their own
  embedding models. Tracked in [`roadmap.md`](../roadmap.md).

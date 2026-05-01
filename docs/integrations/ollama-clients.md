# Ollama-flavoured clients

`npurun serve` implements the Ollama API surface alongside the OpenAI
one. Anything that reads `OLLAMA_HOST` (or accepts an Ollama base URL
in its config) can point at npurun unchanged.

## Smoke test with the Ollama CLI

```powershell
$env:OLLAMA_HOST = "http://127.0.0.1:11435"
ollama list             # talks to npurun, not Ollama
ollama show phi-3.5-mini
ollama run phi-3.5-mini "hello"
```

Note: `ollama pull` is **not** routed — that hits Ollama's registry,
not the npurun model cache. Use `npurun pull` for downloads.

## Endpoints implemented

| Method | Path | Notes |
|---|---|---|
| GET  | `/api/tags` | Loaded model tagged `:latest`. |
| GET  | `/api/version` | Reports npurun semver. |
| POST | `/api/generate` | One-shot completion. NDJSON when `"stream": true`. |
| POST | `/api/chat` | Multi-turn chat. Reuses Genie KV-cache prefix between turns. |
| POST | `/api/show` | Ollama-shaped model info. |
| POST | `/api/delete` | Removes a cached model; refuses if it's the loaded one. |

`pull`, `create`, `push`, and `copy` are not implemented and return 404.

## Clients known to work

- The Ollama CLI itself
- [Page Assist](https://github.com/n4ze3m/page-assist) (browser extension)
- [Enchanted](https://github.com/AugustDev/enchanted) (macOS, but talks HTTP
  so works against a remote Windows host)
- Any TUI that reads `OLLAMA_HOST`

## With auth

`OLLAMA_HOST` doesn't carry a token by convention. If you started
`npurun serve --auth-token <TOKEN>`, most Ollama clients won't pass
it through. Either run loopback-only (the default) or pick a client
that supports a custom Authorization header.

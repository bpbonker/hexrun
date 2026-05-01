# curl + raw HTTP

The smallest possible smoke test. Works on any platform with curl.

## OpenAI surface

```bash
# List the loaded model
curl http://127.0.0.1:11435/v1/models

# One-shot chat
curl http://127.0.0.1:11435/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "phi-3.5-mini",
    "messages": [{"role": "user", "content": "Say hello in five words."}]
  }'

# Streaming via SSE
curl http://127.0.0.1:11435/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -N \
  -d '{
    "model": "phi-3.5-mini",
    "messages": [{"role": "user", "content": "Count from 1 to 5."}],
    "stream": true
  }'
```

## Ollama surface

```bash
# List loaded models (Ollama-shaped)
curl http://127.0.0.1:11435/api/tags

# Streaming /api/generate (NDJSON, not SSE)
curl http://127.0.0.1:11435/api/generate \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "phi-3.5-mini",
    "prompt": "Hello",
    "stream": true
  }'
```

## With auth

If you started `npurun serve --auth-token <TOKEN>`:

```bash
curl http://127.0.0.1:11435/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{ "model": "phi-3.5-mini", "messages": [...] }'
```

## Health check (no auth needed)

```bash
curl http://127.0.0.1:11435/healthz
```

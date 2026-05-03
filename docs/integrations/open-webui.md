# Open WebUI

[Open WebUI](https://github.com/open-webui/open-webui) is a polished
local-first chat UI with multi-conversation history, document upload,
and model switching. Pointing it at `npurun serve` gives you a real
chat interface backed by NPU inference.

## Setup (Docker)

```powershell
# Start npurun with a loaded model
npurun serve --model phi-3.5-mini

# Open WebUI in Docker, talking to npurun on the host
docker run -d `
  --name open-webui `
  -p 3000:8080 `
  -e OPENAI_API_BASE_URL="http://host.docker.internal:11435/v1" `
  -e OPENAI_API_KEY=dummy `
  -e ENABLE_OLLAMA_API=false `
  -e ENABLE_TITLE_GENERATION=false `
  -e ENABLE_TAGS_GENERATION=false `
  -e ENABLE_FOLLOW_UP_GENERATION=false `
  -e ENABLE_AUTOCOMPLETE_GENERATION=false `
  -e ENABLE_SEARCH_QUERY_GENERATION=false `
  -e ENABLE_RETRIEVAL_QUERY_GENERATION=false `
  -v open-webui:/app/backend/data `
  --restart always `
  ghcr.io/open-webui/open-webui:main
```

Open <http://localhost:3000>, sign in (first user becomes the admin),
and the loaded model appears in the model picker.

## Setup (native, no Docker)

```bash
pip install open-webui            # needs Python 3.11 or 3.12
OPENAI_API_BASE_URL=http://127.0.0.1:11435/v1 \
OPENAI_API_KEY=dummy \
ENABLE_OLLAMA_API=false \
DEFAULT_MODELS=qwen3-4b-instruct-2507 \
ENABLE_TITLE_GENERATION=false \
ENABLE_TAGS_GENERATION=false \
ENABLE_FOLLOW_UP_GENERATION=false \
ENABLE_AUTOCOMPLETE_GENERATION=false \
ENABLE_SEARCH_QUERY_GENERATION=false \
ENABLE_RETRIEVAL_QUERY_GENERATION=false \
open-webui serve --host 127.0.0.1 --port 8080
```

The `ENABLE_*_GENERATION=false` flags are important — see the
**Single-user** caveat below.

## With auth

If `npurun serve --auth-token <TOKEN>` is set, pass the token as the
API key:

```powershell
docker run -d `
  -p 3000:8080 `
  -e OPENAI_API_BASE_URL="http://host.docker.internal:11435/v1" `
  -e OPENAI_API_KEY="$TOKEN" `
  ...
```

## Verifying the NPU is actually doing the work

While Open WebUI is generating, watch **Task Manager → Performance →
NPU**. You should see sustained 19–30% utilisation for a 4 GB-class
model. If the NPU column stays at 0% but tokens are still flowing,
something is silently on CPU — see
[`troubleshooting.md`](../troubleshooting.md).

## Caveats

- **Single-user.** `npurun serve` runs one inference at a time —
  Genie owns a single dialog handle whose KV-cache is mutated in place,
  and stacking requests corrupts it (see
  [`architecture.md`](../architecture.md)). The second concurrent
  request gets HTTP 429 with `Retry-After: 1`.

  Open WebUI's defaults will trip this on a single tab: after every
  reply it fires background calls for chat-title generation, tag
  suggestions, follow-up prompts, and autocomplete. If you start
  typing your next message while those are running, you see
  `another inference request is in progress; retry shortly`.

  The setup snippet above disables every background generator. With
  those off, only your visible turns hit npurun and the 429s go
  away. If you want titles back later, expect to wait a beat after
  each reply before sending again.
- **Embeddings (RAG).** Open WebUI's document upload uses its own
  embedding model by default — fine, but it's not running on the NPU.
  npurun's own embeddings endpoint is on the roadmap.
- **Tool / function calling.** Open WebUI exposes a "tools" UI;
  npurun's tool-calling support is tracked as a follow-up. Until
  then, leave tools off in the model settings.

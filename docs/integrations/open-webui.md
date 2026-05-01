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
  -v open-webui:/app/backend/data `
  --restart always `
  ghcr.io/open-webui/open-webui:main
```

Open <http://localhost:3000>, sign in (first user becomes the admin),
and the loaded model appears in the model picker.

## Setup (native, no Docker)

```bash
pip install open-webui
OPENAI_API_BASE_URL=http://127.0.0.1:11435/v1 \
OPENAI_API_KEY=dummy \
open-webui serve
```

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

- **Single-user.** If two browser tabs hit the model simultaneously,
  the second gets an error. `npurun serve` returns HTTP 429 for the
  second request; Open WebUI surfaces that as a chat error.
- **Embeddings (RAG).** Open WebUI's document upload uses its own
  embedding model by default — fine, but it's not running on the NPU.
  npurun's own embeddings endpoint is on the roadmap.
- **Tool / function calling.** Open WebUI exposes a "tools" UI;
  npurun's tool-calling support is tracked as a follow-up. Until
  then, leave tools off in the model settings.

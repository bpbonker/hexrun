# Bundled web chat UI (`/chat`)

`npurun serve` ships with a single-file chat UI baked into the binary,
served at `/chat`. Zero install, no Docker, no pip — just open a
browser.

## Usage

```powershell
npurun serve --model phi-3.5-mini
```

Open <http://127.0.0.1:11435/chat>. Type, hit Enter, watch tokens
stream in.

## What it does

- Streaming chat against `POST /v1/chat/completions` on the same
  origin.
- Multi-turn — every reply is appended to the in-page conversation
  and re-sent on the next turn.
- Live metrics overlay: per-turn TTFT, total time, tokens, tok/s and
  estimated GOP/s; session-level averages in the header bar.
- Optional **tools** toggle exposes two demo functions to the model
  (`get_current_time`, `calc`); the page dispatches them locally
  and feeds the result back via OpenAI-style `role: "tool"`
  messages. Forces non-streaming mode while on.
- Reset / save buttons (save downloads a plain-text transcript).
- Settings persisted in `localStorage`: base URL, model name, bearer
  token, tools toggle.
- Works with `--auth-token` — paste the token in the bearer field.

## What it doesn't do

- No document RAG, no multi-conversation history, no model
  switching. Reach for [Open WebUI](open-webui.md) or
  [AnythingLLM](anythingllm.md) when you want any of that.
- No constrained-sampling JSON mode (the server-side hint applies if
  you call the API directly with `response_format`, but the UI
  doesn't expose it).
- No image / file uploads.
- Tools are a built-in pair for the demo, not a plugin surface.
  Custom tool catalogues belong in your own client.

## Why it exists

Lowest-friction "see this work" path for someone who just installed
npurun and wants a chat box. Docker is in beta on Windows ARM64,
Open WebUI's pip install pulls a heavy dep tree, AnythingLLM is a
real install. `/chat` is one URL.

Source: `tools/web-chat/index.html`. Edit it and rebuild
`npurun-cli` to ship a customised version (the HTML is included via
`include_str!` at compile time).

## CLI alternative — `scripts/chat.ps1`

If you want the same multi-turn streaming chat without a browser at
all, there's a small PowerShell REPL at `scripts/chat.ps1`:

```powershell
pwsh -File scripts\chat.ps1
# you> hello
# npurun> Hi there! ...
```

`/exit` to quit, `/reset` to clear history, `/save` to dump the
transcript, `/sys <prompt>` to set a system prompt. Same streaming
path under the hood, just terminal output instead of HTML.

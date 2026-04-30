# Usage

A working tour of every npurun subcommand. Assumes
[`install.md`](install.md) is done and `npurun version` prints clean output.

## Five-minute tour

```powershell
# 1. Pull a model. Sha256-verified, resumable; goes to %LOCALAPPDATA%\npurun\models.
npurun pull phi-3.5-mini

# 2. One-shot generate. Streams tokens to stdout, timing summary on stderr.
npurun run phi-3.5-mini "Tell me a one-line joke about Snapdragon laptops."

# 3. Or run as a server and point any OpenAI/Ollama client at it.
npurun serve --model phi-3.5-mini
# in another shell:
curl http://127.0.0.1:11435/v1/models
```

Every model-name argument also accepts an `<name>:latest` form (Ollama
convention) and an absolute path to a Genie bundle directory.

## Subcommand reference

### `npurun pull <model>`

Downloads a known bundle from the built-in registry, sha256-verifies the
zip, extracts, and writes `npurun.json`. Resumable via HTTP `Range` if
interrupted.

```powershell
npurun pull phi-3.5-mini             # ~2.1 GB, ~11.7 tok/s on X1E NPU
npurun pull llama-v3-1-8b-instruct   # ~4.5 GB
npurun pull qwen-2-5-7b              # ~4.3 GB, w4a16
```

The registry is currently hardcoded — see
[Phase 5.4 in the roadmap](roadmap.md) for plans to make it remote.
Adding models outside the built-in set is a `npu-convert` workflow, see
[`compatibility.md`](compatibility.md).

### `npurun list`

Prints every cached bundle with its size on disk.

```powershell
PS> npurun list
NAME                       SIZE
phi-3.5-mini               2.1 GB
qwen-2-5-7b                4.3 GB
```

If nothing is cached, prints the cache directory path so you know where
`pull` would land things.

### `npurun show <model> [--profile]`

Pretty-prints the manifest of a cached bundle: arch, quant, context
length, file count, sha256s, chat template. `--profile` adds runtime info
where available.

### `npurun run <model> "<prompt>"`

One-shot generation. Streams tokens to stdout as Genie produces them;
timing summary (TTFT, total, post-TTFT tok/s) goes to stderr so you can
pipe stdout into other tools cleanly.

```powershell
npurun run phi-3.5-mini "Three reasons to learn Rust:"
npurun run phi-3.5-mini "explain the Hexagon NPU architecture in 50 words"
```

Uses the bundle's chat template automatically. For multi-turn
conversation, use `npurun serve` and the chat-completions API — `run` is
deliberately one-shot.

### `npurun bench <model> [--prompt P] [--repeats N] [--no-skip-first]`

Warm-query benchmark. Runs four built-in prompts (or one you supply via
`--prompt`) `--repeats` times each, prints per-prompt and per-repeat
timing, then a warm-summary aggregate that skips the first query (since
that one pays the cold-start cost).

```powershell
npurun bench phi-3.5-mini                      # default 4 prompts × 1 repeat
npurun bench phi-3.5-mini --repeats 3          # 4 prompts × 3 = 12 queries
npurun bench phi-3.5-mini --prompt "hello" --repeats 5
```

Numbers in the README's *Performance, honestly* table came from this
exact harness. See [`benchmarks.md`](benchmarks.md) for raw runs.

### `npurun version`

Three lines: npurun semver, libGenie version (from the linked Genie.lib),
and the QAIRT SDK version + install path. Use this in bug reports.

```text
npurun       0.1.0-rc.2
libGenie     1.17.0
QAIRT SDK    2.45.0  (C:\AAA\Personal\AI\qairt\2.45.0)
```

### `npurun rm <model>`

Deletes a cached bundle. Refuses with HTTP 409 if you ask `npurun serve`
to drop a model it has currently loaded; safe otherwise.

### `npurun serve [--model <name>] [--bind ADDR] [--auth-token TOK] [--no-warmup]`

Starts an OpenAI- and Ollama-compatible HTTP server on `127.0.0.1:11435`
(chosen so it can run alongside Ollama, which uses 11434).

```powershell
# Loopback, single user, default model.
npurun serve --model phi-3.5-mini

# LAN-exposed with bearer auth.
npurun serve --model phi-3.5-mini --bind 0.0.0.0:11435 --auth-token $token

# Skip the post-load warmup query (faster startup, slower first request).
npurun serve --model phi-3.5-mini --no-warmup
```

#### OpenAI-compatible endpoints

| Method | Path | Notes |
|---|---|---|
| GET  | `/v1/models` | Lists the loaded model under both bare and `:latest` names. |
| POST | `/v1/chat/completions` | Blocking JSON or SSE streaming when `"stream": true`. |

#### Ollama-compatible endpoints

| Method | Path | Notes |
|---|---|---|
| GET  | `/api/tags` | Loaded model tagged `:latest`. |
| GET  | `/api/version` | Reports npurun semver. |
| POST | `/api/generate` | One-shot completion. NDJSON when `"stream": true`. |
| POST | `/api/chat` | Multi-turn chat. Reuses the Genie KV-cache prefix between turns. |
| POST | `/api/show` | Ollama-shaped model info (family, quant, context). |
| POST | `/api/delete` | Removes a cached model. Refuses if it's the loaded one. |

#### Health & meta

| Method | Path | Notes |
|---|---|---|
| GET | `/healthz` | JSON: status, model, uptime, auth on/off, version. |
| GET | `/` | Index of available endpoints. |

#### Concurrency, auth, CORS

A single `tokio::sync::Semaphore` permit serializes generation; the
second concurrent request gets `429 Too Many Requests` with
`Retry-After: 1` rather than blocking. `--auth-token` requires
`Authorization: Bearer <token>` on `/v1/*` and `/api/*` (`/healthz` and
`/` stay open). CORS is permissive so browser clients (Open WebUI,
custom UIs) work cross-origin.

`Ctrl+C` triggers a graceful shutdown — in-flight requests finish, then
the listener closes.

### `npurun ps [--addr HOST:PORT] [--auth-token TOK]`

Probes a running `npurun serve` and prints the model it has loaded plus
uptime, auth state, and version. Use it to confirm a server is up and
which model you're talking to.

```powershell
PS> npurun ps
npurun serve at 127.0.0.1:11435
  model:    phi-3.5-mini
  uptime:   12m 4s
  auth:     off
  version:  0.1.0-rc.2
```

If no server is responding, you get a single clean error line, not a
stack trace.

## Open WebUI / generic clients

`npurun serve` speaks both flavours, so any of these work out of the box:

```powershell
# Open WebUI (Docker)
docker run -d -p 3000:8080 \
  -e OPENAI_API_BASE_URL=http://host.docker.internal:11435/v1 \
  -e OPENAI_API_KEY=dummy \
  ghcr.io/open-webui/open-webui:main

# Ollama clients
$env:OLLAMA_HOST = "http://127.0.0.1:11435"
ollama list   # talks to npurun, not to Ollama
```

When `--auth-token` is set, supply it as the `OPENAI_API_KEY` (or
`Authorization: Bearer …` header) in your client.

## Verifying the NPU is actually doing the work

Three checks that **must all agree**:

1. **Task Manager → Performance → NPU** shows sustained utilization
   during a `npurun run` — typically 19–30% for a 4 GB-class model.
2. `npurun show <name>` reports `target_runtime: qnn_dlc` against
   `Snapdragon X Elite CRD`.
3. `npurun bench <name>` produces tokens/sec at least 3× a CPU baseline,
   or above 5 tok/s for 4B-class models.

If the NPU column is at 0% but `npurun run` is still producing text,
you're silently on CPU fallback — file an issue with the output of
`npurun version`.

## Troubleshooting

Anything not behaving? Start with [`troubleshooting.md`](troubleshooting.md)
— it's the running list of every error mode we've hit and how to fix it.

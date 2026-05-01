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

### `npurun run <model> "<prompt>" [--addr HOST:PORT] [--auth-token TOK]`

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

#### Skipping cold-load with `--addr`

Cold-loading the bundle takes 9–11 seconds on Phi 3.5 Mini. If you
already have a `npurun serve` running, pass `--addr <host:port>` to
dispatch the prompt to it via `/v1/chat/completions`. The reply
streams to stdout the same way; the iteration loop drops to under a
second.

```powershell
# Shell A: keep one warm engine resident
npurun serve --model phi-3.5-mini

# Shell B: iterate without paying cold-load each time
npurun run --addr 127.0.0.1:11435 phi-3.5-mini "first prompt"
npurun run --addr 127.0.0.1:11435 phi-3.5-mini "second prompt"
```

`--addr` is also picked up from the `NPURUN_SERVE_ADDR` environment
variable, so you can set it once per shell and drop the flag.

`--auth-token <TOKEN>` mirrors `npurun serve --auth-token` for
LAN-deployed servers. The client validates `/healthz` before
dispatching: if the server has a different model loaded than the one
you asked for, `run` errors instead of letting the server silently
serve whatever it has resident. If the server is busy (HTTP 429), the
client errors immediately rather than retrying — that defeats the
point of bypassing cold-load.

### `npurun bench <model> [--prompt P] [--repeats N] [--no-skip-first] [--ctx N] [--csv PATH]`

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

#### Pinning the context tier with `--ctx`

Genie bundles ship multiple compiled context tiers in the same on-disk
artifact (`cl512`, `cl1024`, `cl2048`, `cl3072`, `cl4096` on the Phi 3.5
Mini bundle, for example). `--ctx` pins the tier for the run; smaller
tiers run faster because the KV cache is smaller. Useful for charting
the speed/context curve.

```powershell
npurun bench phi-3.5-mini --ctx 1024
npurun bench phi-3.5-mini --ctx 4096
```

If the requested value is not one of the bundle's compiled tiers,
`bench` errors with the available list:

```text
Error: context tier 999 not available in this bundle; available tiers: 512, 1024, 2048, 3072, 4096
```

#### Per-query CSV with `--csv`

Append one row per (prompt, repeat) to a CSV. Header columns:

```text
model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft
```

The header is written once when the file is created; subsequent runs
append. Combined with `--ctx`, this is the workflow for tracking
per-tier regressions:

```powershell
npurun bench phi-3.5-mini --ctx 1024 --csv .\phi-1024.csv
npurun bench phi-3.5-mini --ctx 4096 --csv .\phi-4096.csv
```

`--csv` errors fast if the parent directory does not exist, rather than
panicking at write time.

### `npurun version`

Three lines: npurun semver, libGenie version (from the linked Genie.lib),
and the QAIRT SDK version + install path. Use this in bug reports.

```text
npurun       0.1.0-rc.2
libGenie     1.17.0
QAIRT SDK    2.45.0  (C:\AAA\Personal\AI\qairt\2.45.0)
```

### `npurun show-hardware`

Probes the local NPU stack and reports SoC, the Qualcomm Hexagon NPU
PnP entry, the Hexagon architectures the installed QAIRT SDK ships
support for, and the QAIRT + libGenie versions.

```text
PS> npurun show-hardware
SoC:              Snapdragon(R) X 12-core X1E80100 @ 3.40 GHz
NPU:              Snapdragon(R) X Elite - X1E80100 - Qualcomm(R) Hexagon(TM) NPU
Hexagon arch:     hexagon-v66, hexagon-v68, hexagon-v69, hexagon-v73, hexagon-v75, hexagon-v79, hexagon-v81
QAIRT SDK:        2.45.0  (C:\AAA\Personal\AI\qairt\2.45.0)
libGenie:         1.17.0

Status:           Genie API loaded; npurun does not gate on SoC strings.
```

Unlike runtimes that hardcode SoC marketing strings (e.g. AnythingLLM's
QNN engine, see [issues #2962][allm-2962] and [#5129][allm-5129] where
it refuses to start on X Plus / X 10-core variants), npurun does not
match against `Snapdragon(R) X Elite` or any other name. If `libGenie`
loads on your hardware, npurun will try to run a model on it. Failures
surface as real `Genie::Status` errors, not pre-flight rejections.

Use this command in bug reports when filing
[`Compatibility`](compatibility.md) entries or NPU-loading issues.

[allm-2962]: https://github.com/Mintplex-Labs/anything-llm/issues/2962
[allm-5129]: https://github.com/Mintplex-Labs/anything-llm/issues/5129

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
| POST | `/v1/chat/completions` | Blocking JSON or SSE streaming when `"stream": true`. Honours `response_format: {"type": "json_object"}` as a system-prompt hint (not constrained sampling — see below). |

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
| GET | `/chat` | Bundled single-file chat UI. Open in a browser; talks to `/v1/chat/completions` on the same origin. |

#### JSON mode (`response_format: json_object`)

When a client sets `"response_format": {"type": "json_object"}` on a
`/v1/chat/completions` request, npurun augments the system message
(or prepends one) with an instruction asking the model to emit valid
JSON only. This is a **prompt hint, not constrained sampling** — the
model can still produce invalid JSON. Clients should `try { JSON.parse }`
and retry, exactly as against OpenAI's own JSON mode (which has the
same caveat in its docs). Constrained-decoding JSON mode is tracked
in [`roadmap.md`](roadmap.md).

```bash
curl http://127.0.0.1:11435/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "phi-3.5-mini",
    "messages": [{"role": "user", "content": "Return a JSON object with name=\"npurun\" and version=\"0.1.0\""}],
    "response_format": {"type": "json_object"}
  }'
```

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

For AnythingLLM specifically — including why you should use the
Generic OpenAI provider rather than its bundled QNN engine — see
[`integrations/anythingllm.md`](integrations/anythingllm.md).

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

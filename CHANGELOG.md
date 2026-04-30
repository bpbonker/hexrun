# Changelog

All notable changes to hexrun will be documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/spec/v2.0.0.html).

## [0.1.0-dev] — unreleased

### Added — runtime

- Native Rust bindings to Qualcomm Genie via `qnn-sys` (bindgen against
  QAIRT headers) and the safe `qnn::Dialog` wrapper. `Genie.lib` is
  statically linked; `QnnSystem.dll` (which ships without a `.lib`) is
  reserved for future `libloading`-based dynamic dispatch.
- `hexrun-core::Engine` loads a model directory containing
  `hexrun.json` and the bundle's `genie_config.json`, applies the
  manifest's chat template, and runs blocking or streaming inference
  against the held `qnn::Dialog`.
- Manifest schema: `name`, `version`, `arch`, `vocab`, `context`,
  `quant` (with `w4a16`/`w8a16` variants), `qnn_sdk`, `files`, optional
  `chat_template` (`{system}` + `{user}` placeholders), and
  `sha256` map.
- Engine + manifest validation rejects path traversal, absolute paths,
  drive prefixes, malformed sha256, and bad `qnn_sdk` versions.

### Added — CLI (`hexrun`)

- `hexrun pull <name>` — download a known bundle from the built-in
  registry, extract, sha256-verify the zip, and auto-write a manifest.
  Resumable via HTTP `Range`. Built-in registry currently covers
  `phi-3.5-mini`, `llama-v3-1-8b-instruct`, `qwen-2-5-7b`.
- `hexrun list` / `hexrun show <name>` / `hexrun rm <name>` —
  enumerate, inspect, and delete cached models.
- `hexrun run <name> "<prompt>"` — one-shot generation, streams to
  stdout with timing summary on stderr.
- `hexrun bench <name>` — warm-query benchmark; per-prompt
  TTFT/total/gen-time/post-TTFT-tok/s plus an aggregate summary that
  skips the first query.
- `hexrun version` — hexrun + libGenie + QAIRT versions in one shot.
- `hexrun serve --model <name>` — OpenAI- and Ollama-compatible HTTP
  server. Loads the model on startup, runs a small warmup query so the
  first user request is steady-state, then binds the listener.
- TTY-aware progress in `hexrun pull` (indicatif progress bar
  interactively; periodic log lines in non-interactive shells).

### Added — HTTP server

- OpenAI-compatible endpoints: `GET /v1/models`,
  `POST /v1/chat/completions` (blocking JSON + SSE streaming).
- Ollama-compatible endpoints: `GET /api/tags`,
  `POST /api/generate`, `POST /api/chat` (blocking JSON + NDJSON
  streaming).
- `GET /healthz` returns JSON with status, model name, uptime,
  auth-on/off, and version.
- `GET /` index listing the available endpoints.
- Permissive CORS (any origin, GET/POST/OPTIONS, `Authorization` +
  `Content-Type` + `Accept` + `Cache-Control` headers).
- Optional bearer-token auth via `--auth-token <TOKEN>`. When set, all
  `/v1/*` and `/api/*` calls require `Authorization: Bearer <TOKEN>`.
  `/healthz`, `/`, and CORS preflights stay unauthenticated.
- Single-permit `tokio::sync::Semaphore` serializes concurrent
  inference requests; the second concurrent request returns
  `429 Too Many Requests` with `Retry-After: 1` rather than blocking.
- Graceful shutdown on Ctrl+C: in-flight inference completes, then the
  listener closes.
- `--bind 0.0.0.0` prints a clear LAN-exposure warning, with a hint to
  pair with `--auth-token`.

### Added — packaging / hygiene

- `scripts/setup-qnn.ps1` validates a QAIRT install + Hexagon NPU device
  presence.
- `scripts/dev-shell.bat` wraps `vcvarsall.bat arm64`, prepends LLVM
  and (when set) QAIRT bin/lib to PATH, sets `ADSP_LIBRARY_PATH`. Run
  any `cargo` invocation through it.
- `scripts/genie-run.ps1` shells out to `genie-t2t-run.exe` for the
  legacy Phase 0 path (kept for documentation; superseded by `hexrun
  run`).
- README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT, issue and PR
  templates, dependabot config, EditorConfig, rustfmt/clippy/deny
  configs, dual MIT/Apache-2.0 licensing.
- Documentation set: `docs/handoff.md`, `docs/findings.md`,
  `docs/benchmarks.md`, `docs/paper.md`, `docs/architecture.md`,
  `docs/troubleshooting.md`, `docs/compatibility.md`,
  `docs/roadmap.md`.

### Verified on hardware (April 2026)

- **Phi 3.5 Mini (w4a16)** on Snapdragon X Elite NPU: ~11.7 tok/s
  steady-state post-TTFT, 194 ms TTFT, ~9 second cold load. Chat-usable.
- **Qwen 2.5 7B (w8a16)** on Snapdragon X Elite NPU: ~1.9 tok/s
  steady-state post-TTFT, 660 ms TTFT, ~9 second cold load. Slower than
  CPU paths today; the 7B regime is hard on this generation of silicon.
- Tuning experiments (cpu-mask, n-threads, sampler, perf profile)
  produced null results; the single config flag that matters is
  `poll: true` in the QnnHtp section (+36% on Qwen 7B).
- HTTP server end-to-end: OpenAI SSE streaming, Ollama NDJSON
  streaming, bearer-token auth (200/401/200 ladder), CORS preflight,
  HTTP 429 on concurrent requests (one wins, the other gets a clean
  busy response), `/healthz` returns rich JSON.
- **Energy:** Phi 3.5 Mini (w4a16) on the X1E NPU draws ~6.9 W above
  idle to sustain ~11.7 tok/s — about **1.27 J/token**, roughly 2–3×
  more energy-efficient than llama.cpp on the same laptop's CPU.
  Measured via `scripts/energy-bench.ps1` (samples
  `Win32_Battery.DischargeRate` at 2 Hz on battery). Methodology and
  caveats in `docs/benchmarks.md`.

### Notes

- `qnn-sys` and `qnn` crates are excluded from `default-members` until
  `QNN_SDK_ROOT` is set on the build host. This lets contributors check
  out the repo and run `cargo build` without having the proprietary
  SDK. `hexrun-cli` and `hexrun-server` enable the `genie` feature on
  `hexrun-core` to bring qnn into the build.
- `qnn-sys` does not declare a `links` key and does not emit
  `cargo:rustc-link-lib=dylib=QnnSystem` because QAIRT 2.45 ships only
  `QnnSystem.dll` on Windows ARM64 — no static import library. The
  Genie path uses `Genie.lib` (which does ship), so static linking
  works for the LLM runtime.
- The Phase 0 `scripts/smoke_phase0.py` and `scripts/genie-run.ps1`
  paths are kept for documentation; the canonical user-facing
  workflow is now `hexrun pull → run → serve`.

### Pending for v0.1.0

- Multi-turn KV-cache rewind via Genie's `SENTENCE_REWIND` (real chat
  performance on turn 2+).
- README walkthrough screenshot/recording.
- `hex-convert` Python pipeline for HF → bundle conversion (Phase 5).
- Signed Windows MSIX installer + winget manifest (Phase 6).

See `docs/roadmap.md` for the detailed plan.

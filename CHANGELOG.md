# Changelog

All notable changes to npurun will be documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`npurun show-hardware` now reports the NPU driver version + date**.
  Probed via `Get-PnpDeviceProperty` against `DEVPKEY_Device_DriverVersion`
  and `DEVPKEY_Device_DriverDate`. Two machines on the same SoC + QAIRT
  SDK can still fail differently if the OEM-shipped HTP driver diverges
  — this is the field that disambiguates. Filed against issue #12 where
  `Could not create context from binary` fires on a Lenovo Yoga Slim 7x
  with otherwise-identical config to a verified-working machine.
- **Multi-graph bundle compatibility (20× decode speedup)**: bundles
  exported with `prompt_ar128_*` / `token_ar1_*` graph names (Qwen3,
  Qwen2.5-VL, etc.) need `enable-graph-switching: true` in
  `genie_config.json` or libGenie 1.17.0 stays on the prefill graph
  for every decode token. `npurun pull` now injects the flag
  automatically after extraction. Measured on Qwen3-4B-Instruct-2507
  w4a16 / X1E: 0.6 → 11.7 tok/s (qnn example) / 14.9 tok/s
  (`npurun bench`). See [`docs/multi-graph-fix.md`](docs/multi-graph-fix.md).
- **Registry**: three new entries with verified-working precompiled
  bundles — `qwen3-4b-instruct-2507` (4B, 14.9 tok/s),
  `qwen3-4b` (base), `qwen-2-5-vl-7b-instruct` (7B vision-language,
  9.1 tok/s text-only).
- **`npurun bench --duration <SECS>`**: sustained-stress mode. Cycles
  the prompt set until the wall-clock window elapses, then prints
  percentiles, std-dev, and a first-half-vs-second-half degradation
  percent — the single number that catches thermal throttling on
  long runs.

## [0.1.0-rc.3] — 2026-05-01

### Added

- **`npurun show-hardware`**: probe + report SoC, Hexagon NPU PnP entry,
  the Hexagon archs the QAIRT SDK supports, and QAIRT + libGenie
  versions. Does not gate on SoC strings — works on X Elite, X Plus,
  X 10-core. Differentiates from AnythingLLM's QNN engine (issues
  #2962, #5129).
- **`npurun bench --ctx <N>`**: pin a Genie context tier per run;
  validates against the bundle's compiled `clNNNN` set.
- **`npurun bench --csv <PATH>`**: append per-(prompt, repeat) rows
  for tracking per-tier regressions.
- **`npurun run --addr <host:port> [--auth-token TOK]`**: dispatch the
  prompt to a running `npurun serve` via `/v1/chat/completions`,
  skipping the 9-11s cold-load. Reads `NPURUN_SERVE_ADDR`. Validates
  `/healthz` model match; surfaces 429 immediately.
- **OpenAI JSON mode passthrough** (`response_format: {"type":
  "json_object"}`) — augments the system message to instruct the
  model to emit valid JSON only. Prompt hint, not constrained
  sampling. Documented limitation in `docs/usage.md`.
- **Issue template** (YAML form) that requires `npurun show-hardware`
  + `npurun version` output before submission.
- **Client cookbook** under `docs/integrations/` covering curl,
  Python (openai SDK), JavaScript (openai), Open WebUI, AnythingLLM,
  Continue.dev, and Ollama-flavoured clients. Wired into mdBook
  SUMMARY under a new "Integrations" section.
- **Runtime comparison page** (`docs/comparison.md`): npurun NPU vs
  llama.cpp / Ollama / AnythingLLM / NexaSDK / Phi Silica on the
  same X-series laptop. Measured numbers for npurun; cited numbers
  for the others, with sources.
- **ORT-QNN vs libGenie probe** (`docs/findings_ort_vs_genie.md`):
  microbenchmark concludes ORT-QNN is ~2× slower than libGenie on
  this X1E for Phi 3.5 Mini; specula's +19/+39% lift does not
  transfer here. Probe scripts at `scripts/bench-ort-qnn*.py`.
- **X-series rebrand**: capability statements across docs, manifests,
  scripts, README say "Snapdragon X-series" rather than "X Elite".
  X1E preserved in measurement contexts.
- **Architecture doc**: documents the no-concurrency-knob decision
  (one Genie permit, second concurrent request returns 429), citing
  specula.
- **Roadmap Wave G**: open follow-ups for embeddings endpoint,
  smaller chat models in registry, tool calling, constrained-sampling
  JSON mode, remote registry — each with a concrete next action.

### Fixed

- `crates/npurun-server/src/ollama.rs`: moved `enum StreamItem`
  before the `date_tests` module to satisfy
  `clippy::items_after_test_module`.

### Verified

- `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`,
  `cargo test --workspace --exclude qnn --exclude qnn-sys` (57 tests,
  0 failures), `mdbook build`.
- End-to-end smoke on real hardware (X1E80100, libGenie 1.17.0):
  `/v1/models`, `/v1/chat/completions` (blocking + SSE streaming +
  JSON mode + multi-turn KV-cache rewind), `/api/tags`,
  `/api/version`, `/api/chat`, `npurun show-hardware`.

## [0.1.0-dev] — unreleased

### Added — runtime

- Native Rust bindings to Qualcomm Genie via `qnn-sys` (bindgen against
  QAIRT headers) and the safe `qnn::Dialog` wrapper. `Genie.lib` is
  statically linked; `QnnSystem.dll` (which ships without a `.lib`) is
  reserved for future `libloading`-based dynamic dispatch.
- `npurun-core::Engine` loads a model directory containing
  `npurun.json` and the bundle's `genie_config.json`, applies the
  manifest's chat template, and runs blocking or streaming inference
  against the held `qnn::Dialog`.
- Manifest schema: `name`, `version`, `arch`, `vocab`, `context`,
  `quant` (with `w4a16`/`w8a16` variants), `qnn_sdk`, `files`, optional
  `chat_template` (`{system}` + `{user}` placeholders), and
  `sha256` map.
- Engine + manifest validation rejects path traversal, absolute paths,
  drive prefixes, malformed sha256, and bad `qnn_sdk` versions.

### Added — CLI (`npurun`)

- `npurun pull <name>` — download a known bundle from the built-in
  registry, extract, sha256-verify the zip, and auto-write a manifest.
  Resumable via HTTP `Range`. Built-in registry currently covers
  `phi-3.5-mini`, `llama-v3-1-8b-instruct`, `qwen-2-5-7b`.
- `npurun list` / `npurun show <name>` / `npurun rm <name>` —
  enumerate, inspect, and delete cached models.
- `npurun run <name> "<prompt>"` — one-shot generation, streams to
  stdout with timing summary on stderr.
- `npurun bench <name>` — warm-query benchmark; per-prompt
  TTFT/total/gen-time/post-TTFT-tok/s plus an aggregate summary that
  skips the first query.
- `npurun version` — npurun + libGenie + QAIRT versions in one shot.
- `npurun serve --model <name>` — OpenAI- and Ollama-compatible HTTP
  server. Loads the model on startup, runs a small warmup query so the
  first user request is steady-state, then binds the listener.
- TTY-aware progress in `npurun pull` (indicatif progress bar
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
  legacy Phase 0 path (kept for documentation; superseded by `npurun
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

### Added — Ollama parity

- `GET /api/version` returning the running npurun version.
- `POST /api/show` returning Ollama-shaped model info
  (`details.family`, `details.parameter_size`,
  `details.quantization_level`, `template`, `system`, plus a
  `model_info` block with QNN SDK version and context length).
- `POST /api/delete` removing a cached model from disk; refuses with
  HTTP 409 if the named model is the one this server has loaded.
- `<name>:latest` (and any `<name>:<tag>`) is now accepted everywhere a
  model name is — `npurun run`, `npurun show`, `npurun bench`,
  `/v1/chat/completions`, `/api/generate`, `/api/chat`, `/api/show`,
  `/api/delete`. `/v1/models` advertises both the bare and tagged
  forms; `/api/tags` advertises the `:latest`-tagged form to match
  Ollama clients' expectations.
- `npurun ps` now actually does something: probes `GET /healthz` on
  `--addr` (default `127.0.0.1:11435`) and prints the loaded model,
  uptime, auth state, and version. Optional `--auth-token` for
  servers running with bearer-token auth.

### Notes

- `qnn-sys` and `qnn` crates are excluded from `default-members` until
  `QNN_SDK_ROOT` is set on the build host. This lets contributors check
  out the repo and run `cargo build` without having the proprietary
  SDK. `npurun-cli` and `npurun-server` enable the `genie` feature on
  `npurun-core` to bring qnn into the build.
- `qnn-sys` does not declare a `links` key and does not emit
  `cargo:rustc-link-lib=dylib=QnnSystem` because QAIRT 2.45 ships only
  `QnnSystem.dll` on Windows ARM64 — no static import library. The
  Genie path uses `Genie.lib` (which does ship), so static linking
  works for the LLM runtime.
- The Phase 0 `scripts/smoke_phase0.py` and `scripts/genie-run.ps1`
  paths are kept for documentation; the canonical user-facing
  workflow is now `npurun pull → run → serve`.

### Added — multi-turn chat

- `qnn::Dialog::query_streaming_with` accepts an input
  `SentenceCode`. The Phase-4 `query_streaming` keeps its
  COMPLETE-only behaviour and is now a thin shim over the new
  function.
- `ChatTemplate` gained two optional fields, `assistant_turn` and
  `next_user_turn`, plus a `wrap_chat(messages)` method that builds
  a full multi-turn transcript from a `[ChatMessage]` slice. Single-
  turn bundles without the new fields fall back to wrapping the most
  recent user message via `template` (preserving Phase 4 behaviour).
- New canonical types `npurun_core::ChatMessage` and
  `npurun_core::ChatRole` for crossing the HTTP-handler / engine
  boundary.
- `Engine::generate_chat` and `Engine::generate_chat_streaming` accept
  a full message history. The first call on a fresh dialog is sent
  with `SentenceCode::Complete` (populates the KV cache); every
  subsequent call uses `SentenceCode::Rewind` so Genie matches the
  transcript prefix against the cache and re-prefills only the new
  tokens. `Engine::reset_chat` drops the cache and forces the next
  call back to `Complete`.
- The HTTP server's `/v1/chat/completions` and `/api/chat` now thread
  the full `messages` array through the engine instead of extracting
  only the latest user message — multi-turn conversations actually
  preserve context across turns. `/api/generate` is unchanged
  (single-shot completions).
- The built-in registry's chat templates carry the new
  `assistant_turn` / `next_user_turn` fields for Phi 3, Llama 3, and
  Qwen 2.5; new pulls produce multi-turn-capable manifests.
- Verified on hardware against `phi-3.5-mini`: turn-2 questions about
  facts established in turn 1 are answered correctly, on both
  blocking and streaming paths, on both OpenAI and Ollama surfaces.

### Added — `npu-convert` (Phase 5 starter)

- Python sidecar at `python/npu-convert/` with three subcommands:
  - `npu-convert manifest --model-dir <dir> --bundle-dir <bundle>
    --name <slug>` reads the Genie bundle's `genie_config.json`,
    sniffs `arch` and `quant` from the bundle directory name (or
    accepts explicit overrides), looks up the matching chat template
    (Phi 3 / Llama 3 / Qwen 2.5 patterns ship as defaults), walks the
    bundle for sha256 sealing, and writes a `npurun.json`. Pure
    Python, ARM64-friendly. Smoke-tested against the real local
    Phi 3.5 Mini bundle — produces a manifest the Rust runtime loads
    cleanly with all 8 file sha256s captured.
  - `npu-convert inspect <bundle-or-manifest>` pretty-prints either a
    full npurun manifest (with file sizes + chat template + on-by-
    default sha256 verification) or a raw Genie bundle (showing what
    a manifest would contain).
  - `npu-convert export <slug> --output <dir>` orchestrates the heavy
    HF -> ONNX -> AI-Hub-cloud-compile -> Genie bundle pipeline by
    shelling out to `qai-hub-models`'s per-model export script, then
    chains into `manifest`. Curated recipes for `phi-3.5-mini`,
    `llama-v3-1-8b-instruct`, `qwen-2-5-7b`. Requires
    `QAI_HUB_API_TOKEN`, x64 Python, and 30-90 minutes of cloud
    compile time per model. `--skip-compile` re-runs only the
    manifest step when the bundle is already on disk.
- Optional dependency split in `pyproject.toml`: the lightweight
  default (`click`, `rich`) covers `manifest` and `inspect` for
  ARM64; the heavy ONNX / `qai-hub-models` stack is gated behind
  `pip install -e ".[export]"` for x64 Python.
- 8 passing pytest cases covering manifest emission against
  synthesized Genie-bundle layouts: arch / quant sniffing,
  sha256 correctness, chat-template selection, explicit overrides,
  rejection of bundles outside `model_dir`, missing
  `genie_config.json`, missing `tokenizer.json`, and unknown arch
  without an override.

### Added — Phase 6 starter (release ergonomics)

- **winget manifest** at `manifests/b/bpbonker/npurun/0.1.0-rc.1/`,
  three YAML files (version, en-US locale, installer) following the
  v1.6.0 schema. Installer type is `zip` with a portable nested
  `npurun.exe` and a `Commands: [npurun]` alias, so winget can install
  the existing GitHub-release zip with no code signing required —
  validated by `winget validate`. End-users can already
  `winget install --manifest manifests\b\bpbonker\npurun\0.1.0-rc.1`
  off a clone of the repo. Public-catalog submission to
  `microsoft/winget-pkgs` is gated on a signed installer (Phase 6
  final).
- **Tag-triggered release workflow** at
  `.github/workflows/release.yml`. Runs on `v*` tag push and
  `workflow_dispatch`, gated `if: false` until a self-hosted ARM64
  runner with QAIRT is enrolled. Builds zip + MSIX, signs the MSIX
  if `MSIX_CERT_THUMBPRINT` secret is configured, opens a GitHub
  release with all four artifacts (auto-detects `-rc/-alpha/-beta`
  suffix and marks pre-release).
- **CI matrix expanded:** `python-test` (pytest on `npu-convert`)
  and `winget-validate` (validates every published manifest dir on
  push) jobs added to `ci.yml` alongside the existing fmt/clippy/
  build/test/python-lint set.
- **`scripts/dev-cert.ps1`** for development MSIX signing.
  Generates a self-signed code-signing cert in
  `CurrentUser\My`, attempts to import it into
  `LocalMachine\TrustedPeople` (admin needed) so a signed dev MSIX
  installs by double-click without developer mode. `-List` shows
  existing dev certs; `-Remove <thumb>` deletes one. Pairs with
  `scripts/build-msix.ps1 -CertThumbprint <thumb>`.
- **`docs/release.md`** — copy-paste runbook for cutting a release:
  pre-flight, version bump, build artifacts, smoke-test, tag + push,
  GitHub release, winget manifest update, code-signing options
  (dev self-signed, Azure Trusted Signing, DigiCert/Sectigo, EV cert),
  and CI matrix overview. The literal flow rather than the
  aspirational one.

### Added — Phase 6.9 (docs site)

- **mdBook docs site.** `book.toml` (rebranded from the leftover hexrun
  config) + `docs/SUMMARY.md` (table of contents) + `docs/index.md`
  (landing page) build the existing `docs/*.md` set into a static site
  under `book/`. Local preview via `mdbook serve --open`.
- **`.github/workflows/docs.yml`** runs `mdbook build` on every push
  to `main` and deploys to GitHub Pages via `actions/deploy-pages@v4`;
  PRs get build-only verification so a broken `SUMMARY.md` fails the
  PR before merge. Path-filtered to `docs/**`, `book.toml`, and the
  workflow itself so unrelated commits don't burn CI minutes.
- One-time enablement per repo: Settings → Pages → Source: GitHub
  Actions. After the first successful run on `main` the site is live
  at `https://bpbonker.github.io/npurun/`.
- README's `## Documentation` section now leads with a link to the
  rendered site (search + navigable TOC); per-chapter links to
  GitHub-rendered markdown remain for repo browsers.

### Changed — docs cross-links

- Fixed broken cross-references that would have rendered as 404s on
  the docs site:
  - `docs/architecture.md` no longer points at a personal Claude
    plan path that lives outside the repo.
  - `docs/handoff.md` "pair it with" list now uses repo-relative
    links into the docs site (with absolute GitHub URLs for the
    README and CHANGELOG, which sit outside the `docs/` tree); the
    stale references to personal Claude state files were dropped.
  - `docs/compatibility.md` link to the issue template is now an
    absolute GitHub URL — relative `../.github/...` paths cannot
    resolve from inside the rendered site.

### Fixed — bench dialog reuse

- `npurun bench` was reusing one Genie dialog across queries without
  resetting the KV cache between them. The second query ran in a
  context that still contained turn 1's tokens, generation didn't
  terminate cleanly (one observed run produced 321 tokens for a
  "briefly explain" prompt and bled into what looked like the next
  prompt's template), and Genie eventually returned `ERROR_QUERY_FAILED`
  (status -6 / Error 1003) on a subsequent query. `bench_model` now
  calls `engine.reset_dialog()` between queries; renamed
  `Engine::reset_chat` → `Engine::reset_dialog` since the same call
  applies to single-shot benchmark loops, not just chat resets, and
  the doc comment now spells out the contamination failure mode it
  prevents. Verified end-to-end on Phi 3.5 Mini: 4-query bench
  completes cleanly, ~12.7 tok/s post-TTFT, ~105 ms TTFT.

### Verified — full end-to-end smoke (2026-05-01)

Against the rebuilt rc.2 binary on Snapdragon X Elite NPU:

- `npurun pull phi-3.5-mini` — 2.08 GB in 250 s, sha256-verified, manifest written.
- `npurun list` / `npurun show` — manifest fields render correctly (phi3, W4A16, ctx 4096).
- `npurun run` — 209 ms total for "What is 2+2?", 14.3 chunks/s, terminated cleanly on EOS.
- `npurun bench` (post-fix) — 4 prompts, no errors, **12.7 tok/s post-TTFT, 105 ms TTFT** (slightly better than the README's headline 11.7 tok/s / 200 ms numbers, which stay in place as conservative).
- `npurun serve --model phi-3.5-mini` — `/healthz`, `/v1/models`, `/v1/chat/completions` (blocking + SSE), `/api/tags`, `/api/version`, `/api/chat` (blocking) all responsive and well-formed; CORS headers present; concurrent second request returns clean **HTTP 429 with `retry-after: 1`** instead of blocking.
- `npurun ps` against the running server — reports correct status, model, uptime, auth state, version.

### Fixed — Ollama-compat timestamp date stuck at 1970-01-01

- `now_iso8601` in `npurun-server::ollama` was throwing the day count
  away (`let _ = days;`) and slapping a literal `1970-01-01` prefix
  onto the time-of-day, so every `created_at` / `modified_at` field
  on `/api/tags` / `/api/chat` / `/api/generate` rendered with the
  Unix-epoch date even though the time was correct. Replaced the
  placeholder formatter with Howard Hinnant's `days_from_civil`
  inverse (compact public-domain algorithm; no `chrono`/`time`
  dependency). Four unit tests cover the epoch origin, leap-day 2024,
  current-era 2026, and the century non-leap-year edge case (2100).
  Verified live against a running server: `modified_at` now reports
  `2026-04-30T21:53:32Z` instead of `1970-01-01T21:41:58Z`.

### Added — specula catch-up follow-ups (2026-05-01)

Four small wins from the briefing in `docs/specula_followups.md`, picked
up as one drop. Briefing references their respective items.

#### Architecture: documented the no-concurrency-knob decision (Item 3)

`docs/architecture.md` now has a "Why one permit instead of N" section
explaining that the single-permit semaphore in `npurun serve` is the
load-bearing consequence of libGenie owning a single in-place dialog
handle, not a placeholder waiting to be lifted. Cites specula's
session-22 finding ("NPU absent (Genie has no concurrency knob)") as
external corroboration.

#### Docs: generation-portability sweep (Item 4)

Softened "Snapdragon X Elite" capability statements across the README,
`book.toml`, MSIX/winget metadata, install.md, index.md, handoff.md,
release.md, roadmap.md, compatibility.md, CONTRIBUTING.md, the
`npurun-cli` clap `about` and Cargo.toml descriptions, and crate-level
doc comments. The framing is now "Snapdragon X-series Windows-on-ARM
laptops" for capability surfaces; X1E stays in the load-bearing
measurement, verification, and Qualcomm-AI-Hub-device-name spots
(benchmarks, energy figures, `Snapdragon X Elite CRD` compile flag,
hardware verification rows, `paper.md` / `findings.md` historical
reports). `compatibility.md` gained a top-of-file note that the table's
numbers were measured on X Elite and that the project targets the
X-series broadly.

#### `npurun bench --ctx <N>` and `--csv <PATH>` (Item 1)

`npurun bench` now accepts:

- `--ctx <N>` to pin the Genie context tier for a run. Validated
  against the bundle's compiled `clNNNN` tiers (parsed from `ctx-bins`
  filenames in `genie_config.json`); errors with the available list if
  the requested value isn't compiled in. The override is threaded
  through `EngineConfig.ctx`, applied by patching
  `dialog.context.size` in the loaded JSON, and consumed via a new
  `qnn::Dialog::from_config_json_in_dir` constructor. The bench
  header now shows the resolved (overridden) ctx.
- `--csv <PATH>` to append one row per (prompt, repeat) with columns
  `model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft`.
  Header is written once on file creation; subsequent runs append.
  Errors fast if the parent directory is missing rather than
  panicking on `File::create`.

Smoke-test on cached Phi 3.5 Mini bundle:
`npurun bench phi-3.5-mini --ctx 1024 --csv phi-1024.csv --prompt "hi"
--repeats 2` produced a header + two data rows; a second run against
the same path appended cleanly without re-emitting the header. `--ctx
999` errored with `available tiers: 512, 1024, 2048, 3072, 4096`.

Specula corroboration: their per-ctx scaling tables on Qwen3-4B are
the same shape; this is what the new `--csv` output is built to
support across npurun versions.

#### `npurun run --addr <host:port>` (Item 2)

`npurun run` now accepts `--addr <host:port>` (also picked up from the
`NPURUN_SERVE_ADDR` environment variable) and `--auth-token <TOK>`.
When `--addr` is set, the prompt is POSTed to a running `npurun
serve`'s `/v1/chat/completions` with `stream: true`, and the
incoming SSE chunks are streamed to stdout as they arrive. Skips the
9–11 s bundle cold-load (~2 s end-to-end on Phi 3.5 Mini in the
smoke test, vs ~10 s cold).

Pre-flight via `/healthz`: if the server has a different model loaded
than the one the user asked for, `run --addr` errors with the
mismatch and a hint to either restart serve, point at the right
server, or drop `--addr` to load locally. HTTP 429 from a busy server
is surfaced as a clean error rather than retried (retrying would
defeat the point of bypassing cold-load). `<name>:latest` references
are stripped before comparison.

Specula corroboration: their session-21 measurement showed amortizing
HTP context init via a sidecar process gave +51% on AR1 workloads.
`npurun serve` already gives that to HTTP clients; this closes the gap
for the iterative `npurun run` UX too.

### Investigated — ORT-QNN vs libGenie probe (2026-05-01)

Microbenchmark of per-stage QNN HTP execution latency on
[`llmware/phi-3.5-onnx-qnn`](https://huggingface.co/llmware/phi-3.5-onnx-qnn)
vs the libGenie path npurun ships, on this X1E80100. Driver:
`scripts/bench-ort-qnn-microbench.py` against `onnxruntime-qnn` 2.1.0
(plugin EP via `add_provider_for_devices`). Full writeup in
`docs/findings_ort_vs_genie.md`.

**Result:** ORT-QNN summed across the 4 ar128 prefill shards (~250 ms)
and 4 ar1 decode shards (~149 ms) runs roughly **2× slower** than
libGenie's reported full-pipeline TTFT (~110 ms) and per-token decode
(~80 ms) on this hardware. The +19% TG / +39% PP lift specula
[reported on Qwen3-4B/X2](https://github.com/hotschmoe/specula) does
**not** transfer to Phi-3.5-mini/X1E in this microbenchmark
configuration.

The full-pipeline equivalent was blocked: PyPI / ORT-Nightly
`onnxruntime-genai` wheels (stable `0.13.1`, nightly
`0.13.0.dev20260402`) are not compiled with `--use_qnn`. Source-built
QNN-enabled ort-genai wheels would change the picture; the unfinished
`scripts/bench-ort-qnn.py` is parked for that path. See
`docs/findings_ort_vs_genie.md` for caveats and reproduction.

Implication for `Backend::Ort` (currently `EngineError::FeatureDisabled`
in `npurun-core`): don't budget the port expecting a free perf lift
on X1E. Re-test on X2 silicon when available before committing.

### Added — `npurun show-hardware` (2026-05-01)

New CLI subcommand that probes and prints the local NPU stack:
SoC marketing name (via `Get-CimInstance Win32_Processor`), the
Qualcomm Hexagon NPU PnP entry (`Get-PnpDevice`), the Hexagon
architectures the installed QAIRT SDK ships support for (walking
`<QNN_SDK_ROOT>/lib/hexagon-vNN/`), and the QAIRT + libGenie
versions. Does **not** gate on SoC strings — the differentiator
versus AnythingLLM's QNN engine, which refuses to start on X Plus /
X 10-core variants (issues #2962, #5129). If libGenie loads, npurun
runs.

Use this in bug reports and Compatibility entries.

Implementation: `crates/npurun-cli/src/main.rs::show_hardware` and
helper probes (`detect_soc`, `detect_npu_pnp`, `detect_hexagon_arch`).
Filesystem-based unit tests for `detect_hexagon_arch` cover the
present and missing-layout paths.

### Added — AnythingLLM-as-client integration recipe (2026-05-01)

`docs/integrations/anythingllm.md`: configure AnythingLLM's Generic
OpenAI provider against a running `npurun serve` to bypass
AnythingLLM's hardcoded SoC string check while keeping its workspace
UI, document RAG, and agents. Wired into `docs/SUMMARY.md` under a
new "Integrations" section. Cross-linked from `docs/usage.md`.

### Pending for v0.1.0
- README walkthrough screenshot/recording.
- `npu-convert` Python pipeline for HF → bundle conversion (Phase 5).
- Signed Windows MSIX installer + winget manifest (Phase 6).

See `docs/roadmap.md` for the detailed plan.

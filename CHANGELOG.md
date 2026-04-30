# Changelog

All notable changes to npurun will be documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/spec/v2.0.0.html).

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

### Pending for v0.1.0
- README walkthrough screenshot/recording.
- `npu-convert` Python pipeline for HF → bundle conversion (Phase 5).
- Signed Windows MSIX installer + winget manifest (Phase 6).

See `docs/roadmap.md` for the detailed plan.

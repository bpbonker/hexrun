# Roadmap to v0.1.0

The path from "developer's-laptop working tool" (where we are now) to a
real public v0.1.0 release. Items are grouped into waves; each wave is
designed to produce one or more committable units. Time estimates assume
familiarity with the codebase.

This document is the source of truth for what's left. Update it as items
land or get cut.

---

## Wave A — Complete the CLI surface (~45 min)

Goal: every command we want users to type works.

| # | Item | Effort | Notes |
|---|---|---:|---|
| A1 | `npurun bench <model>` subcommand | 20 min | Fold the existing `qwen-bench`/`phi-bench` examples into the CLI; print the warm-summary table |
| A2 | `npurun --version` reports libGenie + QAIRT versions | 10 min | Pull `qnn::api_version()` and read `$QNN_SDK_ROOT/sdk.yaml`; useful in bug reports |
| A3 | TTY-aware pull progress | 15 min | Detect `is_terminal`; non-TTY → periodic log lines instead of indicatif clobber |

## Wave B — Pull integrity (~75 min)

Goal: a flaky network or a corrupted bundle doesn't ruin a user's day.

| # | Item | Effort | Notes |
|---|---|---:|---|
| B1 | sha256 verification on pull | 30 min | Compute during download, persist to manifest; skip on `--insecure` |
| B2 | Resume interrupted downloads via HTTP Range | 45 min | If `.pull.zip` exists, send `Range: bytes=<size>-`; fallback to fresh on 416 or no-range support |

## Wave C — Server polish (~50 min)

Goal: the server behaves predictably under abuse and shutdown.

| # | Item | Effort | Notes |
|---|---|---:|---|
| C1 | Graceful shutdown on Ctrl+C | 20 min | `axum::serve(...).with_graceful_shutdown(signal::ctrl_c())`; drain in-flight |
| C2 | HTTP 429 with `Retry-After` when busy | 30 min | `try_lock` instead of blocking on the engine mutex; if taken, return 429 |

## Wave D — Docs (~40 min)

Goal: README + handoff describe the actual current product, not the dev-time scaffolding.

| # | Item | Effort | Notes |
|---|---|---:|---|
| D1 | README rewrite | 25 min | Lead with `pull → run → serve`; drop the manual zip-flow narrative |
| D2 | `docs/handoff.md` rewrite | 10 min | Update to reflect Phase 2-4 plus LAN-safety drop |
| D3 | CHANGELOG.md updates | 5 min | Capture today's commits in `[0.1.0-dev]` |

## Wave E — Headline numbers (✅ done 2026-04-30)

Goal: quantify the NPU's energy efficiency claim with measured data.

| # | Item | Effort | Notes |
|---|---|---:|---|
| E1 | Energy measurement script | done | `scripts/energy-bench.ps1` samples `Win32_Battery.DischargeRate` at 2 Hz on battery; computes idle vs. busy delta, total inference energy, joules per token. Phi 3.5 Mini result: **~1.27 J/token at 6.9 W delta**. Captured in `docs/benchmarks.md`. |

## Wave F — Real chat performance (done 2026-05-01)

Goal: turn 2+ of a multi-turn conversation is fast.

| # | Item | Effort | Notes |
|---|---|---:|---|
| F1 | Multi-turn KV-cache rewind via `GENIE_DIALOG_SENTENCE_REWIND` | done | First chat call on a fresh dialog goes through with `SentenceCode::Complete`, populating the KV cache. Every subsequent call passes the full transcript with `SentenceCode::Rewind`; Genie matches the prefix and re-prefills only the new tokens. Multi-turn fields (`assistant_turn`, `next_user_turn`) added to `ChatTemplate`; OpenAI + Ollama chat surfaces thread the full messages array through `Engine::generate_chat_streaming`. Verified on Phi 3.5 Mini end-to-end. |

## Phase 5 — `npu-convert` (starter shipped 2026-05-01)

| # | Item | Effort | Notes |
|---|---|---:|---|
| 5.1 | `npu-convert manifest` writes a `npurun.json` for any local Genie bundle | done | Reads `genie_config.json`, sniffs arch + quant, looks up chat template, sha256-seals the bundle. Pure Python, ARM64-friendly. 8 tests. Smoke-tested against the real Phi 3.5 Mini bundle. |
| 5.2 | `npu-convert inspect` pretty-prints + verifies | done | Handles both `npurun.json` manifests (with sha256 verify) and raw Genie bundles. |
| 5.3 | `npu-convert export` orchestrates a fresh AI Hub compile | done (orchestrator) | Curated recipes for `phi-3.5-mini`, `llama-v3-1-8b-instruct`, `qwen-2-5-7b`. Shells out to `qai-hub-models`'s per-model export script and chains into `manifest`. End-user runs it, takes 30-90 minutes per model. Not exercised in CI. |
| 5.4 | Remote registry beyond the hardcoded list | open | The `npurun pull` registry is still hardcoded. Future work: a JSON index hosted somewhere, signed by the publisher, fetched at pull time. |

## Phase 6 — release ergonomics (starter shipped 2026-05-01)

| # | Item | Effort | Notes |
|---|---|---:|---|
| 6.1 | winget manifest | done | `manifests/b/bpbonker/npurun/0.1.0-rc.1/` with version + locale + installer YAMLs (zip-installer type, portable nested binary, `Commands: [npurun]`). Validated by `winget validate`. Installs today via `winget install --manifest <dir>` against a clone. |
| 6.2 | Tag-triggered release workflow | scaffolded | `.github/workflows/release.yml` builds zip + MSIX, signs MSIX if `MSIX_CERT_THUMBPRINT` secret set, opens release with all four artifacts. `if: false` until self-hosted ARM64 runner is enrolled. |
| 6.3 | CI matrix expansion | done | `python-test` (pytest on `npu-convert`) and `winget-validate` jobs added alongside the existing fmt/clippy/build/test/python-lint set. |
| 6.4 | Dev MSIX signing | done | `scripts/dev-cert.ps1` generates a self-signed code-signing cert in `CurrentUser\My`, imports into `LocalMachine\TrustedPeople` (admin), pairs with `scripts/build-msix.ps1 -CertThumbprint`. |
| 6.5 | Release runbook | done | `docs/release.md` is the copy-paste flow: pre-flight, version bump, build, smoke-test, tag + push, GitHub release, winget update, code-signing options. |
| 6.6 | Production code signing | open | Need a real cert — Azure Trusted Signing ($10/mo), DigiCert/Sectigo standard ($200-400/yr), or EV cert. Once provisioned, expose thumbprint to CI as a secret. |
| 6.7 | Public winget catalog submission | gated on 6.6 | PR to `microsoft/winget-pkgs`. Reviewers prefer signed installers. |
| 6.8 | Self-hosted ARM64 CI runner | open | Enrol the dev laptop (or a dedicated X-series box) as a self-hosted runner with `QNN_SDK_ROOT` set as a secret; flip `release.yml` and `build-arm64-with-qnn` from `if: false`. |
| 6.9 | Docs site | done | mdBook (`book.toml` + `docs/SUMMARY.md` + `docs/index.md`) builds the existing `docs/*.md` set into a static site. `.github/workflows/docs.yml` runs `mdbook build` on every push to `main` and deploys to GitHub Pages via `actions/deploy-pages`; PRs get build-only verification so a broken `SUMMARY.md` fails the PR. Site goes live at `https://bpbonker.github.io/npurun/` once Settings → Pages → Source is set to "GitHub Actions" once. |

## Wave G — Client breadth (open, captures 2026-05-01 follow-ups)

Goal: anything an OpenAI-compatible client expects, npurun provides.
Items in this wave are mostly blocked on either Genie-compiled bundles
that don't exist yet, or on real engineering effort that needs its own
session.

| # | Item | Status | Next action |
|---|---|---|---|
| G1 | `/v1/embeddings` endpoint | blocked on bundle | Convert `bge-small-en-v1.5` (33 M params, 512 ctx) or `e5-small-v2` to a Genie context binary via `npu-convert`. The conversion path exists end-to-end for LLMs; embeddings are a new shape of model and `qai-hub-models` doesn't ship a recipe — needs a direct ONNX → Genie path through the Qualcomm AI Hub UI. Once the bundle exists, the server side is ~half a day: a new `Engine::embed` method that runs prefill-only and returns the pooled hidden state. |
| G2 | Smaller chat models in registry | blocked on bundles | Add Llama 3.2 1B and Gemma 2 2B (or Llama 3.2 3B) to the built-in registry. Both are public on HF, neither has a Qualcomm-shipped Genie bundle today. Workflow: `npu-convert export <id>` against AI Hub (30–90 min each), then add registry entries with sha256 + URL. Tracked as part of [issue G2 once filed]. |
| G3 | Tool calling (`tools` + `tool_choice`) | engineering, multi-day | Real tool-calling needs (1) a model-specific chat-template extension that injects tool schemas in the prompt, (2) a streaming-aware parser that detects the model's tool-call syntax and emits OpenAI-shaped `tool_calls`, (3) round-trip handling of `role: "tool"` messages on follow-up turns. Per-model: Llama 3.1 emits `<\|python_tag\|>{...}`, Qwen 2.5 emits `<tool_call>...</tool_call>` XML, Phi 3.5 Mini was not trained for tools (return 400 with a clear message). Gate by model-name match in `npurun-server`; punt to a future commit. |
| G4 | Constrained-sampling JSON mode | engineering | Current JSON mode (`response_format: {"type": "json_object"}`) is a prompt hint only — see `crates/npurun-server/src/openai.rs::augment_for_json_mode`. Real JSON mode would mask logits during sampling so the model can only emit tokens consistent with valid JSON. Genie exposes a token-by-token streaming API but not a logit-bias hook today; needs Qualcomm-side support or a wrap-and-resample pattern that's ugly enough to think hard about before building. |
| G5 | Remote model registry (was 5.4) | open | The `npurun pull` registry is still hardcoded in `crates/npurun-registry/src/lib.rs`. Replace with a signed JSON index fetched from a known URL (with the current hardcoded set as the bootstrap fallback). Rough shape: `https://npurun.io/registry/v1.json` → `{ name → { url, sha256, size, qnn_sdk } }`, signed with a project key. |

## Wave H — Bigger contexts and bigger models (next stage, in flight 2026-05-02)

Goal: push beyond the 4096-context default that all current registry
bundles ship with, and validate the export pipeline against bigger
weights.

| # | Item | Status | Notes |
|---|---|---|---|
| H1 | Qwen3-4B Instruct 2507 at 32K context | blocked on memory | Local x64 export OOMs during ONNX consolidation: `tensor.raw_data = data_file.read()` fails on the 17 GB unpacked w4a16 blob even with 16 GB RAM + 40 GB pagefile. Path forward is the cloud Linux x86 build farm, see Wave I. The 17.7 GB asset itself downloaded fine via `scripts/qwen3-4b-download-loop.sh` (resilient curl resume loop). |
| H2 | Llama 3.1 8B Instruct w4a16 | queued | Has `DEFAULT_W4A16` prequantized checkpoint, no AIMET-ONNX needed. Same export path as Qwen3-4B but bigger weights — will OOM harder on the 16 GB laptop. Bundle target: 4 ctx tiers (4096, 8192, 16384, 32768). Then registry entry + bench + multi-graph injection check. |
| H3 | Qwen 2.5 7B w4a16 (replace the legacy w8a16 row) | gated on bundle | The current registry's `qwen-2-5-7b` is an old w8a16 self-export (~0.9–1.9 tok/s). Once a w4a16 multi-graph variant is available — either a Qualcomm precompiled or a local export — replace and retire the slow row. |
| H4 | More context tiers in registry manifests | engineering | Pull-time, the manifest currently records a single `context_length`. Update `npurun-registry::Manifest` and `npurun pull` to record the full `clNNNN` tier set baked into the bundle so `npurun bench --ctx <N>` can pin against it without re-parsing `genie_config.json`. |

## Wave I — Cloud Linux x86 build farm (open)

Goal: a clean separation between the Windows-on-ARM laptop (where
npurun runs) and the Linux x86 box (where exports get built). The
laptop is too memory-constrained for >4B w4a16 ONNX consolidation; AI
Hub Models' AIMET pipeline is Linux-only anyway.

| # | Item | Status | Notes |
|---|---|---|---|
| I1 | Portable export bundle (`scripts/wsl-export/`) | starter shipped 2026-05-02 | `setup.sh` builds a Python 3.10 venv with the discovered pin set (transformers 4.51.0, torch 2.4.1, sentencepiece, accelerate, conditional aimet-onnx). `run-export.sh` wraps `python -m qai_hub_models.models.<id>.export` with the standard flag set. Drop the dir on a WSL2 Ubuntu 22.04 box (or any x86 Linux) with `qai-hub configure` set, run setup once, run exports many times. |
| I2 | Self-hosted Linux x86 GitHub Actions runner | open | Pair with the existing Wave 6.8 (self-hosted ARM64 runner) so CI can both build (`linux-x86`) and validate-on-target (`win-arm64`). |
| I3 | Bundle artifact upload to a CDN | open | Once exports succeed on the build farm, upload the `.zip` bundles to a CDN (or a private S3) with sha256 manifests so `npurun pull` can fetch them. Pairs with Wave G5 (remote registry). |

## Beyond v0.1.0 — explicitly deferred

- **Snapdragon X2 support** when hardware ships.

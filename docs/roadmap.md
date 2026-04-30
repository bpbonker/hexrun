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
| 6.8 | Self-hosted ARM64 CI runner | open | Enrol the dev laptop (or a dedicated X1E box) as a self-hosted runner with `QNN_SDK_ROOT` set as a secret; flip `release.yml` and `build-arm64-with-qnn` from `if: false`. |
| 6.9 | Docs site | done | mdBook (`book.toml` + `docs/SUMMARY.md` + `docs/index.md`) builds the existing `docs/*.md` set into a static site. `.github/workflows/docs.yml` runs `mdbook build` on every push to `main` and deploys to GitHub Pages via `actions/deploy-pages`; PRs get build-only verification so a broken `SUMMARY.md` fails the PR. Site goes live at `https://bpbonker.github.io/npurun/` once Settings → Pages → Source is set to "GitHub Actions" once. |

## Beyond v0.1.0 — explicitly deferred

- **Snapdragon X2 support** when hardware ships.
- **Remote registry** (Phase 5.4) — the `npurun pull` index is still hardcoded; a signed JSON index hosted somewhere is the next step toward unbundling that.

## Execution plan for tonight

Crank through **A → B → C → D**, commit each wave separately. If energy
is still in the tank, do **E**. **F (multi-turn) is the next session's
headline item** — too big to risk leaving half-done.

Stretch goal: ship a tagged `0.1.0-rc.1` after Wave D so the repo has a
referenceable release point even without the installer story.

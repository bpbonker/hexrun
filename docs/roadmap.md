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
| A1 | `hexrun bench <model>` subcommand | 20 min | Fold the existing `qwen-bench`/`phi-bench` examples into the CLI; print the warm-summary table |
| A2 | `hexrun --version` reports libGenie + QAIRT versions | 10 min | Pull `qnn::api_version()` and read `$QNN_SDK_ROOT/sdk.yaml`; useful in bug reports |
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

## Phase 5 — `hex-convert` (starter shipped 2026-05-01)

| # | Item | Effort | Notes |
|---|---|---:|---|
| 5.1 | `hex-convert manifest` writes a `hexrun.json` for any local Genie bundle | done | Reads `genie_config.json`, sniffs arch + quant, looks up chat template, sha256-seals the bundle. Pure Python, ARM64-friendly. 8 tests. Smoke-tested against the real Phi 3.5 Mini bundle. |
| 5.2 | `hex-convert inspect` pretty-prints + verifies | done | Handles both `hexrun.json` manifests (with sha256 verify) and raw Genie bundles. |
| 5.3 | `hex-convert export` orchestrates a fresh AI Hub compile | done (orchestrator) | Curated recipes for `phi-3.5-mini`, `llama-v3-1-8b-instruct`, `qwen-2-5-7b`. Shells out to `qai-hub-models`'s per-model export script and chains into `manifest`. End-user runs it, takes 30-90 minutes per model. Not exercised in CI. |
| 5.4 | Remote registry beyond the hardcoded list | open | The `hexrun pull` registry is still hardcoded. Future work: a JSON index hosted somewhere, signed by the publisher, fetched at pull time. |

## Beyond v0.1.0 — explicitly deferred

- **Phase 6: release prep**. Signed Windows MSIX installer, winget manifest, signed CI matrix on a self-hosted ARM64 runner, docs site (github.io). Multi-day.
- **Snapdragon X2 support** when hardware ships.
- **Remote registry** (Phase 5.4 above) — the `hexrun pull` index is still hardcoded; a signed JSON index hosted somewhere is the next step toward unbundling that.

## Execution plan for tonight

Crank through **A → B → C → D**, commit each wave separately. If energy
is still in the tank, do **E**. **F (multi-turn) is the next session's
headline item** — too big to risk leaving half-done.

Stretch goal: ship a tagged `0.1.0-rc.1` after Wave D so the repo has a
referenceable release point even without the installer story.

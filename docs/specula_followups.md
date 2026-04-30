# specula follow-ups: catch-up implementation context

Implementation briefings for the gaps where npurun lags
[specula](https://github.com/hotschmoe/specula). Each section is
self-contained — meant to be handed to a fresh agent or contributor
who hasn't followed the conversation that produced this list.

## Background

[specula](https://github.com/hotschmoe/specula) is a sister research
project on the next-generation Snapdragon X2 silicon doing
speculative-decoding work. Their `npu_engine/` has built things on the
same QAIRT / Hexagon stack that npurun targets. Their findings have
already validated several of npurun's design decisions and surfaced
several gaps. This doc captures the gaps that are *catchable* —
small-to-medium engineering tasks that close the gap without
multi-week R&D.

What is **not** in this doc: the ORT-QNN-vs-Genie perf-probe (separate
investigation, weeks of work, gated on its own decision), full
speculative-decoding support, multi-device heterogeneous compute, or
any of specula's research-frontier work. Those belong in `roadmap.md`
under post-v0.1.0 work, not here.

## Phi 3.5 Mini bundle layout (load-bearing reference)

Audited 2026-05-01. The cached `phi-3.5-mini` bundle's
`genie_config.json` shows it ships a single weight-shared model with
**five context tiers AND two prefill modes packed into the same four
context binaries**:

```text
weight_sharing_model_ar128_ar1_cl512_cl1024_cl2048_cl3072_cl4096_{1,2,3,4}_of_4.serialized.bin
```

- `ar128` = autoregressive batched-128 prefill (one tile of 128 tokens
  prefilled per call). Best for prompt processing on long inputs.
- `ar1` = autoregressive single-token prefill. Best for chat / short
  prompts.
- `cl512` … `cl4096` = the available context-length tiers.
- `{1..4}_of_4` = four-partition split for HTP memory.

**Implication:** any per-ctx-tier or per-prefill-mode work happens in
the libGenie config layer, not by re-compiling the model. Genie picks
the right binary based on requested context size and prompt length.
This is why specula's per-ctx scaling tables work: same bundle, just
different `n-context` in the dialog config.

`size: 4096` in the dialog's `context` block is what asks Genie to use
the cl4096 binary. To pin lower tiers, write the dialog config with a
smaller `size`.

---

## Item 1: `npurun bench --ctx <N>` and `--csv <path>`

### What

Add two flags to `npurun bench`:

- `--ctx <N>`: pin the context tier for the run. If the bundle
  doesn't have that tier, error with the list of available tiers.
- `--csv <path>`: emit one CSV row per (prompt, repeat) so users
  can track regression across versions. Existing pretty-printed
  output stays as the default to stderr.

### Why

specula publishes per-ctx tables (e.g. Qwen3-4B drops from 27.8 t/s
@ ctx=512 to 20.3 t/s @ ctx=4096, sublinear). npurun's bench uses the
bundle-default ctx only — users can't see the speed/context curve.

CSV output lets us actually answer "did the latest rc regress on Phi
3.5 Mini?" without scraping commit messages or markdown tables.

### Where

- [crates/npurun-cli/src/main.rs](crates/npurun-cli/src/main.rs) — `Cmd::Bench { … }` and `bench_model(…)`.
- [crates/npurun-core/src/engine.rs](crates/npurun-core/src/engine.rs) — `EngineConfig`. The `ctx` field needs to be threaded into the Genie dialog-config builder.
- [crates/qnn/src/genie.rs](crates/qnn/src/genie.rs) — wherever the dialog JSON is constructed for `Dialog::new` / `Dialog::create`. Look for the `"context": { "size": ... }` write.

### Approach

1. Add `ctx: Option<u32>` to `EngineConfig`. Default `None` = use the
   bundle's manifest-declared context.
2. In the Genie dialog-config builder, when `Some(n)`, emit
   `"context": { "size": n, ... }` instead of the manifest default.
3. Add `--ctx` to `Cmd::Bench` (and optionally `Cmd::Run` /
   `Cmd::Serve` for symmetry, but bench is the priority).
4. Add `--csv <path>`. If passed, on every query record, append a row:
   `model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft`.
   Write a header row if the file is new.
5. Update `docs/usage.md` examples.

### Gotchas

- **Tier validation matters.** Genie will accept any `size` value but
  silently bind to the next-larger compiled tier (or fail at load).
  Either parse `genie_config.json`'s `ctx-bins` filename to extract
  available `clNNNN` tokens (regex `cl(\d+)`), or just pass the user's
  value through and let Genie error — but the latter UX is worse.
- The bench's `Engine::reset_dialog()` between queries must still be
  called (we just fixed this in commit `9611c7b`); don't regress it.
- If `--csv` is passed but the path's parent dir doesn't exist, fail
  fast with a clear error rather than panicking on `File::create`.

### Acceptance

- `npurun bench phi-3.5-mini --ctx 1024` runs and reports faster
  numbers than the default `--ctx 4096`.
- `npurun bench phi-3.5-mini --ctx 999` errors with a list of valid
  tiers (512/1024/2048/3072/4096 for Phi).
- `npurun bench phi-3.5-mini --csv out.csv` produces a header + N
  data rows where N = prompts × repeats.
- All four bench unit tests still pass; new tests for the CSV writer.

---

## Item 2: `npurun run --addr <host:port>` (skip cold load)

### What

When `--addr` is set on `npurun run`, post the prompt to a running
`npurun serve`'s `/v1/chat/completions` instead of cold-loading a
fresh `Engine`. Skips the 9–11 s bundle load.

### Why

specula's measurement: amortizing the ~15 s HTP context init across
calls via a sidecar gives **51% faster on AR1 workloads**. We get
this implicitly with `npurun serve` already, but `npurun run`
(the CLI's iterative-prototype command) pays cold-load every
invocation. Closes the UX gap.

### Where

- [crates/npurun-cli/src/main.rs](crates/npurun-cli/src/main.rs) — `Cmd::Run { … }` and `run_model(…)`.
- The HTTP-client plumbing already exists for `npurun ps`
  (search for `Cmd::Ps` and the `reqwest`/`ureq` call against
  `/healthz`). Reuse the same client.

### Approach

1. Add `--addr <host:port>` and `--auth-token <TOKEN>` flags to
   `Cmd::Run` (mirror the `Cmd::Ps` shape).
2. If `--addr` is set:
   - POST to `http://<addr>/v1/chat/completions` with the prompt
     wrapped as `[{"role":"user","content":"<prompt>"}]`,
     `stream=true`.
   - Stream-print SSE chunks to stdout as they arrive (same shape as
     today's local-dialog streaming callback).
   - Honour `--auth-token` by adding `Authorization: Bearer <token>`.
3. If `--addr` is not set: existing behaviour (cold-load Engine).
4. Optional: env-var fallback — `NPURUN_SERVE_ADDR=127.0.0.1:11435`.

### Gotchas

- The remote server may have a *different* model loaded than the
  `<model>` argument. Three options:
  1. Ignore `<model>` when `--addr` is set, just use whatever's loaded.
  2. Validate via `/healthz` first, error if mismatch.
  3. Pass `<model>` through as the request's `"model"` field and let
     the server decide (it currently accepts the loaded model's name).
  Pick (2) — it's the least-surprising UX.
- If the server isn't responding, fall back gracefully with a clear
  error ("no server at <addr>; either start one with `npurun serve` or
  drop --addr to load locally").
- Don't accidentally retry on 429 — that defeats the point.

### Acceptance

- `npurun serve --model phi-3.5-mini` in one shell.
- `npurun run --addr 127.0.0.1:11435 phi-3.5-mini "hi"` in another
  returns in <1 s (vs 9–11 s cold-load) with the same output as
  cold-load.
- `npurun run --addr 127.0.0.1:11435 wrong-model "hi"` errors
  clearly without crashing.

---

## Item 3: Document the no-concurrency-knob decision

### What

Add a paragraph to `docs/architecture.md` explaining *why* `npurun
serve` uses a single-permit semaphore + HTTP 429 instead of
multiplexing. Cite specula as external corroboration.

### Why

Future contributors will look at the semaphore and try to "fix" it.
It's not a bug — libGenie has no concurrency knob; specula confirmed
the same when they had to spawn N processes for ORT-QNN concurrency.
Documenting this saves a session of someone trying.

### Where

- [docs/architecture.md](docs/architecture.md) — append a section.

### Approach

A short subsection under whatever heading covers the server, roughly:

> ### Why one permit instead of N
>
> `npurun serve` holds an `Arc<Engine>` behind a
> `tokio::sync::Semaphore` with a single permit. Concurrent requests
> beyond one return HTTP 429 with `Retry-After: 1` rather than
> queuing. This is **not** a placeholder — it's the load-bearing
> consequence of libGenie owning a single dialog handle whose KV
> cache state is mutated in-place per query. Genie does not expose a
> concurrency knob; running multiple inferences against one dialog
> would corrupt the cache (we already saw the failure mode in the
> bench dialog-reuse bug, fixed in `9611c7b`).
>
> The sister project [specula](https://github.com/hotschmoe/specula)
> confirms the same constraint: their session-22 work explicitly
> notes "NPU absent (Genie has no concurrency knob)" and they had
> to spawn N processes for any ORT-QNN concurrency benchmark.
>
> If we ever need real multi-tenant serving the path is process
> fan-out (each replica owns its own dialog) or an ORT-QNN backend
> using a multi-session pool — not adding permits to this semaphore.

### Acceptance

- Section lives in `docs/architecture.md`.
- mdBook builds clean; the link to specula renders.

---

## Item 4: Generation portability sweep

### What

Audit the codebase for hardcoded "X1E" / "Snapdragon X Elite" /
"Hexagon v73" assertions and soften them. The runtime is already
silicon-agnostic in principle (libGenie abstracts the silicon); the
README, architecture doc, and the registry's "verified on" notes are
where we lock ourselves into one generation.

### Why

User wants npurun forward-compatible to X2 (and beyond). Cheaper to
do this framing pass now than retrofit when X2 hardware lands.

### Where (audit results)

Per `grep -lri "X1E\|X Elite\|hexagon-v73"`:

- README.md — multiple "Snapdragon X Elite" callouts in the headline
  and prerequisites table.
- CHANGELOG.md — historical entries; **leave as-is**, history is
  history.
- crates/npurun-cli/src/main.rs — likely a help-text reference.
- crates/npurun-core/src/engine.rs — possibly a comment.
- docs/index.md, docs/usage.md, docs/install.md, docs/handoff.md,
  docs/compatibility.md, docs/roadmap.md — user-facing docs.
- book.toml — site description.
- manifests/b/bpbonker/npurun/0.1.0-rc.2/*.yaml — winget metadata.
- installer/AppxManifest.xml — Description string.
- python/npu-convert/* — sidecar README.
- crates/qnn/examples/* — example names.

### Approach

The framing principle: **target = "Snapdragon X-series laptops with
Hexagon NPU + QAIRT".** Mention X1E specifically only when stating
*verified* numbers (where the hardware is load-bearing for the
claim). Don't mention X1E in capability statements ("npurun runs
on", "supports").

Concrete edits per file type:

- **README headline**: change "for Snapdragon X Elite" to "for
  Snapdragon X-series Windows-on-ARM laptops". Keep the X1E
  callout in the *Performance* table because those numbers are X1E
  specifically.
- **README prerequisites table**: change "Snapdragon X Elite or X
  Plus laptop" to "Snapdragon X-series laptop (X Elite, X Plus, or
  X2 — anything with a Hexagon NPU and QAIRT support)".
- **docs/install.md**: same softening for the "won't run on x64
  silicon" line.
- **docs/usage.md, docs/index.md, docs/handoff.md**: replace
  "Snapdragon X Elite" with "Snapdragon X-series" except where
  citing measured numbers.
- **book.toml description**, **MSIX Description**, **winget
  ShortDescription / Description / Tags**: same.
- **crates/qnn/examples/**: rename or repurpose `phi-bench.rs` /
  `qwen-bench.rs` only if they have hardcoded X1E asserts.
  Probably leave example names alone.
- **compatibility.md / roadmap.md**: these explicitly track
  hardware — leave the X1E specifics where they're accurate, just
  add a top-of-file note that npurun targets the X-series broadly.

### Gotchas

- **Don't blanket-replace.** Where we say "we measured 11.7 tok/s on
  X Elite", that's load-bearing precision — leave it.
- The `compatibility.md` doc explicitly lists per-arch results.
  That's the right place to enumerate; don't redact, just clarify
  framing.
- **Don't claim X2 support before we have it.** "X2 likely works,
  not yet verified" is the right register.

### Acceptance

- `grep -ril "Snapdragon X Elite"` returns roughly the
  measurement/citation files only, not the capability/marketing
  surfaces.
- README and docs read as "X-series, primarily verified on X1E"
  rather than "X1E only."
- mdBook still builds; no broken cross-references.

---

## Implementation order suggestion

If picked up by a single agent or contributor in one focused session:

1. **Item 3 (concurrency doc)** first — 30 min, no code, gets the
   architecture written down before someone refactors away from it.
2. **Item 4 (portability sweep)** next — half day, all docs/markdown,
   no test risk.
3. **Item 1 (bench --ctx + --csv)** — half-to-full day, real code,
   real tests. Ship as one PR.
4. **Item 2 (run --addr)** — half day, small surface, mostly HTTP-
   client plumbing reused from `ps`.

Total: ~1.5 days at a relaxed pace, all four shippable as one
release-cycle drop.

## Verification when done

After all four:

```powershell
# 1. Source-quality
scripts\dev-shell.bat cargo fmt --all -- --check
scripts\dev-shell.bat cargo clippy --workspace --exclude qnn --exclude qnn-sys -- -D warnings
scripts\dev-shell.bat cargo test --workspace --exclude qnn --exclude qnn-sys

# 2. Doc-site
mdbook build

# 3. Live smoke against a real bundle (Phi 3.5 Mini)
scripts\dev-shell.bat target\release\npurun.exe bench phi-3.5-mini --ctx 1024 --csv .\phi-1024.csv
scripts\dev-shell.bat target\release\npurun.exe bench phi-3.5-mini --ctx 4096 --csv .\phi-4096.csv
scripts\dev-shell.bat target\release\npurun.exe serve --model phi-3.5-mini    # one shell
scripts\dev-shell.bat target\release\npurun.exe run --addr 127.0.0.1:11435 phi-3.5-mini "hello"   # other shell, returns <1s

# 4. Rebuild MSIX with new binaries
scripts\build-msix.ps1 -SkipBuild
```

Update CHANGELOG with one entry per item, citing this doc and the
specula sources where relevant.

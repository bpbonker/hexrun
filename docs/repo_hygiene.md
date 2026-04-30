# Repo hygiene

What stays in the repo, what gets cleaned, and when. Inspired by
specula's [three-bucket model](https://github.com/hotschmoe/specula/blob/main/docs/repo_hygiene.md),
adapted to npurun's smaller scope (a runtime, not a research lab).

The goal isn't minimalism. It's: **someone clones the repo and finds
their way around in under two minutes.**

## What's in scope

This doc covers the *contents* of the repo. Per-machine state (cached
models in `$LOCALAPPDATA\npurun\models\`, the QAIRT install at
`$QNN_SDK_ROOT`, signing certs in the user's cert store) is the user's
to manage — the CLI provides `npurun rm <name>` and the install docs
cover uninstall.

## The three buckets

| bucket | location | meaning |
|---|---|---|
| **keep** | top level — `crates/`, `docs/`, `scripts/`, `python/`, `manifests/`, `installer/AppxManifest.xml`, repo-root markdown | source of truth, hand-authored, irreplaceable |
| **regenerate** | `target/`, `dist/`, `book/`, `installer/Assets/`, `python/**/dist/`, `python/**/*.egg-info/` | byproducts of the build; gitignored; rebuild on demand |
| **archive** | `docs/archive/` (does not exist yet — create on first use) | superseded markdown that's still historically useful |

Markdown never gets hard-deleted. Move it to `docs/archive/` instead.
Research prose is cheap to keep and expensive to re-derive.

## Per-directory rules

### `crates/` — source

- Hand-authored Rust. Every file is load-bearing.
- A crate that's no longer used: delete it from the workspace (don't
  leave it sitting unreferenced "in case").
- Test fixtures live next to the code that uses them.

### `docs/` — markdown

Keep at top level:
- Active reference docs (`architecture.md`, `troubleshooting.md`,
  `compatibility.md`, `release.md`, `repo_hygiene.md`).
- User-facing docs (`install.md`, `usage.md`, `index.md`,
  `SUMMARY.md`).
- Living planning docs (`roadmap.md`, `handoff.md`).
- Measurement records (`benchmarks.md`).
- Long-form writeups (`findings.md`, `paper.md`).

Move to `docs/archive/`:
- Closed phase-by-phase docs once their findings are folded into a
  retrospective or the next phase's doc.
- Investigations that resolved one way and won't be revisited (the
  outcome is captured elsewhere; the investigation itself is
  history).
- One-off communications (upstream issue drafts, ask-docs for
  external collaborators) once the ask is answered.

Before adding a new `docs/*.md`, check whether it fits into an
existing doc. New files are free to create and expensive in six
months when three docs cover overlapping ground.

### `scripts/` — PowerShell + Python helpers

- Every script in here is something a contributor or release engineer
  runs. Delete unused ones aggressively — orphan scripts rot fast.
- A script that's been superseded: delete it. Its replacement is the
  documentation for what it does.

### `dist/` — release artifacts

- Always gitignored.
- After cutting a release: keep the *current* version's artifacts on
  disk locally if you want them; everything older can `rm -rf`. The
  authoritative copy lives on the GitHub release page.
- The MSIX staging dir (`*-msix-staging/`) is rebuilt every time
  `scripts\build-msix.ps1` runs — never check in, never preserve.
- Stale artifacts from a prior version (e.g. leftover `hexrun-*`
  files after a rename) cause confusion at install time. Sweep them
  when you cut the next release.

### `installer/Assets/` — generated icons

- Always gitignored. `scripts\build-msix.ps1` regenerates the four
  PNGs (StoreLogo, Square44x44, Square150x150, Wide310x150) on first
  run.
- The script *skips* regeneration if the file exists, so when the
  brand changes (e.g. text rendered onto the icon) you must
  `rm installer/Assets/*.png` to force a fresh render.

### `book/` — mdBook output

- Always gitignored. Built by CI (`.github/workflows/docs.yml`) and
  published to GitHub Pages.
- Local preview via `mdbook serve --open`; the resulting `book/` dir
  is yours to ignore or delete at will.

### `target/` — Cargo build cache

- Always gitignored. Cargo manages it.
- Aggressive `cargo clean` is fine at any time; it just costs a
  rebuild.

### `python/npu-convert/` — Python sidecar

- Source: `pyproject.toml`, `npu_convert/`, `tests/`.
- Generated: `dist/`, `*.egg-info/`, `.pytest_cache/`, `.ruff_cache/`,
  `__pycache__/` — all gitignored.

## When to tidy

- **Just before / after cutting a release.** Sweep `dist/` of stale
  versions. Confirm `installer/Assets/` doesn't have stale icons
  from a prior brand. Check `dist/<version>-msix-staging/` is clean.
- **When a phase closes.** Move its phase-specific docs to
  `docs/archive/` if their findings are now in a retro or in a
  promoted reference doc. Update `docs/SUMMARY.md` so the docs site
  stays clean.
- **Before adding a non-trivial new artifact.** Ten minutes of
  classifying what's already there beats a year of "what is this
  file for?" later.

Skip tidying mid-investigation. The cost of deleting a file you
needed is much higher than the cost of a messy working tree. Tidy at
the *boundary* between phases, not inside them.

## What never goes in the repo

- Models or model bundles (multi-GB; live in `$LOCALAPPDATA\npurun\
  models\` per-machine).
- The QAIRT SDK (proprietary, non-redistributable, lives at
  `$QNN_SDK_ROOT` per-machine).
- Code-signing certificates or private keys (`scripts\dev-cert.ps1`
  generates per-machine; production certs live in the appropriate
  signing service, never on disk).
- API tokens, bearer secrets, `.env` files. Anything resembling a
  credential goes in `.gitignore` proactively.

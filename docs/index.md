# npurun

**NPU-first local LLM runtime for Snapdragon X Elite (Windows on ARM).**

This is the npurun documentation site. The headline numbers, prerequisites,
and quick-start live in the project [README] — the chapters here are the
deeper reference material that supports them.

[README]: https://github.com/bpbonker/npurun#readme

## Where to start

- Just want to install it? **[Installation](install.md)** covers MSIX,
  zip, and build-from-source paths plus the QAIRT runtime requirement.
- Already installed? **[Usage](usage.md)** is the working tour of every
  subcommand (`pull` / `run` / `serve` / `bench` / `ps` / …) plus the
  HTTP endpoint reference for OpenAI- and Ollama-compatible clients.
- New here? Read **[Handoff & current status](handoff.md)** for a one-page
  picture of what works today, then **[Roadmap](roadmap.md)** for what's left
  before v0.1.0.
- Curious about the numbers? **[Benchmarks](benchmarks.md)** has the raw
  measurements (Phi 3.5 Mini at ~11.7 tok/s and ~1.27 J/token on the X1E
  NPU) and the methodology behind them.
- Hit a wall during setup? **[Troubleshooting](troubleshooting.md)** is a
  growing list of every failure mode we've seen and the fix.
- Cutting a release? **[Release runbook](release.md)** is the literal
  copy-paste flow.

## How npurun is built

- **[Architecture](architecture.md)** — the load-bearing design decisions:
  how `qnn-sys` / `qnn` / `npurun-core` / `npurun-cli` / `npurun-server`
  fit together, why the server holds a single `Arc<Engine>` behind a
  one-permit semaphore, how multi-turn chat reuses the Genie KV cache.
- **[Findings](findings.md)** — the engineering blog post: the path
  from "popular OSS LLM tools all run CPU-only on this laptop" to
  "native Rust over libGenie, NPU-verified end-to-end."
- **[Paper](paper.md)** — formal experience-report writeup of the
  same work, in academic-paper form.
- **[Model compatibility](compatibility.md)** — which HuggingFace
  LLMs we've successfully converted with `npu-convert`, and which
  patterns are known-broken on this generation of silicon.

## Project links

- [Source on GitHub](https://github.com/bpbonker/npurun)
- [Issue tracker](https://github.com/bpbonker/npurun/issues)
- [Releases](https://github.com/bpbonker/npurun/releases)

Dual-licensed under MIT or Apache-2.0 at your option.

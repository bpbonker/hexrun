# Contributing to npurun

Thanks for considering a contribution. npurun is early-stage and most useful
contributions right now are:

- **Phase 0 / 1 testing on real hardware** — try the setup script on your
  Snapdragon X-series device (X Elite, X Plus, X2 — anything with a
  Hexagon NPU and QAIRT support) and file an issue if it doesn't work.
- **Op-coverage reports** — convert a model with `npu-convert` and tell us
  which ops fall back to CPU on the HTP. We track these in
  [docs/compatibility.md](docs/compatibility.md).
- **Performance reports** — tokens/sec, first-token latency, NPU utilization
  on different chips and Windows versions.

## Ground rules

1. **Robustness over features.** A new feature without tests, error
   handling, and `tracing` instrumentation will be sent back. See
   [docs/architecture.md](docs/architecture.md) for the bar.
2. **No regressions in NPU utilization.** A change that silently shifts
   work to the CPU is a bug, not an optimization. The CI benchmark suite
   (Phase 6) will enforce this.
3. **Three-checks rule for NPU usage.** Task Manager NPU column,
   `QNN_LOG_LEVEL=PROFILE` confirming HTP execution, and >3× CPU
   tokens/sec — all three before claiming "runs on NPU."
4. **License compatibility.** Code is dual-licensed MIT / Apache-2.0. Don't
   import GPL-only deps. Run `cargo deny check licenses` locally.
5. **No vendored QNN SDK.** It's not redistributable. Build scripts must
   read it from `QNN_SDK_ROOT`.

## Dev setup

See the README "Prerequisites" and "Phase 0 walkthrough" sections.

```powershell
# After QNN SDK + Rust + LLVM are installed:
pwsh -File .\scripts\setup-qnn.ps1
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Pull requests

- Title: imperative summary under 70 chars (`add: qnn::Capabilities probe`).
- Body: what + why, not just what. If it's a bug fix, link the issue.
- Tests required for new logic. `unimplemented!()` is not a test.
- Squash-merge by default; rebase onto `main` before merging.

## Reporting bugs

Use the "Bug report" issue template. Include:

- Hardware (X Elite / X Plus / X2 / specific laptop)
- Windows version (`winver`)
- QNN SDK version + HTP driver version (from `setup-qnn.ps1` output)
- `cargo --version`, `rustc --version`
- The smallest reproduction you can manage
- `RUST_LOG=npurun=debug,qnn=debug` log of the failing run, redacted as needed

Silent failures are the worst class of bug in this domain — please attach
the QNN profile log if you have one (`QNN_LOG_LEVEL=PROFILE`, then
`QnnHtp.log` from your run).

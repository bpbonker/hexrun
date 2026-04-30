# Changelog

All notable changes to hexrun will be documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[SemVer](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial Cargo workspace and Python sidecar layout (qnn-sys, qnn,
  hexrun-core, hexrun-registry, hexrun-server, hexrun-cli, hex-convert).
- `scripts/setup-qnn.ps1` — validates a QNN SDK install and the
  Hexagon NPU device.
- Repo hygiene: CONTRIBUTING, SECURITY, CHANGELOG, CoC, issue/PR
  templates, EditorConfig, rustfmt + clippy + cargo-deny configs.
- CI workflow scaffolding (fmt, clippy, build, test, ruff). Self-hosted
  ARM64 runner job stubbed for full QNN-enabled build.
- Project documentation in `docs/` and an architecture overview.

### Notes
- `qnn-sys` and `qnn` crates are excluded from `default-members` until
  `QNN_SDK_ROOT` is set on the build host. This lets contributors check
  out the repo and run `cargo build` without having the proprietary SDK.
- `qnn-sys` does not declare a `links` key and does not emit
  `cargo:rustc-link-lib=dylib=QnnSystem` because QAIRT 2.45 ships only
  `QnnSystem.dll` on Windows ARM64 — no static import library. Runtime
  loading via `libloading` is planned for Phase 1.

### Phase 0 — verified 2026-04-30

End-to-end NPU inference works on this hardware. Qwen 2.5 7B Instruct
(w8a16 quantized) running on the Hexagon NPU on Snapdragon X Elite,
producing coherent text. Task Manager NPU column shows ~19% sustained
utilization with 4.9 GB shared memory in use during generation; CPU
~12%; bundle compiled by Qualcomm AI Hub's cloud against a real
Snapdragon X Elite chip.

Bundle path: `C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite\`
SDK: QAIRT 2.45.0. Runtime: Genie 1.17.0 via `genie-t2t-run.exe`.

See `docs/handoff.md` for the full reproduction recipe.

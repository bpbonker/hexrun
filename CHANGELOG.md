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

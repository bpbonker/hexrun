# Security policy

## Reporting a vulnerability

Please do **not** open a public issue for security-sensitive reports. Email
the maintainers directly with details and a reproduction. We aim to
acknowledge within 72 hours.

## Scope

In scope for security reports:

- Code execution from a malicious model file or manifest (path traversal,
  archive extraction, deserialization, allocator abuse).
- Authentication / authorization bypasses in `npurun serve`.
- Memory unsafety in `qnn` / `qnn-sys` (FFI boundary).
- TOCTOU / atomicity bugs in `npurun-registry` cache writes.
- HTTP request smuggling, header injection, SSE injection in the server.
- Insecure defaults (e.g. `npurun serve` binding to `0.0.0.0` without
  authentication when documentation implies otherwise).

Out of scope:

- Issues only reproducible against the proprietary QNN SDK runtime itself —
  report those to Qualcomm.
- DoS via legitimate but expensive inputs (e.g. huge model files); we'll
  treat those as resource-management bugs, not security issues.

## Hardening choices that are not bugs

- `npurun serve` defaults to `127.0.0.1`. Binding to a non-loopback
  interface without an auth token is intentionally allowed but
  documented as user-managed risk.
- Model files are downloaded from the configured registry over HTTPS and
  sha256-verified against the manifest. The integrity boundary is the
  manifest signature (Phase 6 — manifest signing planned).

## Summary

<!-- 1-3 bullets: what changed and why -->

## Test plan

<!-- Markdown checklist -->
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Ran on hardware (specify):
- [ ] NPU utilization unchanged or improved (Task Manager + `QnnHtp.log`)
- [ ] No new `unwrap()` / `expect()` on user-reachable paths
- [ ] Public APIs documented

## Linked issues

<!-- Closes #123 -->

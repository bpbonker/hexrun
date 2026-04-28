---
name: Bug report
about: Something that doesn't work the way it should
title: "[bug] "
labels: bug
---

## What happened

<!-- Describe the failure. If "the NPU isn't being used", explain how you checked. -->

## What you expected

## Reproduction

```powershell
# Smallest commands that trigger the bug
```

## Environment

- Hardware: <!-- e.g. Surface Pro 11, Snapdragon X1E80100 -->
- Windows: <!-- output of `winver`, plus 24H2/26H1 etc. -->
- QNN SDK: <!-- output of `setup-qnn.ps1` -->
- HTP driver: <!-- "Snapdragon ... Hexagon NPU" version from Device Manager -->
- `rustc --version`:
- `cargo --version`:
- hexrun commit / version:

## Logs

<details><summary>RUST_LOG=hexrun=debug,qnn=debug output</summary>

```
<!-- paste here -->
```

</details>

<details><summary>QnnHtp.log (if relevant)</summary>

```
<!-- paste here, redact paths if needed -->
```

</details>

# Installation

Three install paths, fastest first. All require a Snapdragon X Elite (or X
Plus) laptop running Windows 11 24H2+ on ARM64 — npurun is NPU-first and
won't run on x64 silicon.

## 1. MSIX installer (recommended)

Download `npurun-<version>-aarch64-windows.msix` from the [latest
release](https://github.com/bpbonker/npurun/releases). Then **one** of:

### A. Double-click install (signed builds)

Public-release MSIXs are signed by a CA Windows already trusts — double-click
the `.msix` and click Install. No further setup. (Phase 6 final; not yet
shipped — early `0.1.0-rc.*` builds are unsigned. See B or C below.)

### B. Developer-mode install (unsigned dev builds)

For unsigned `.msix` files (anything you build yourself out of the repo, or
an `0.1.0-rc.*` pre-release):

```powershell
# One-time: enable developer mode (no admin needed)
# Settings -> Privacy & Security -> For developers -> Developer Mode = ON

Add-AppxPackage -Path .\npurun-0.1.0-rc.2-aarch64-windows.msix
```

If you skip the developer-mode step `Add-AppxPackage` fails with
`HRESULT: 0x800B0100, No signature was present in the subject.`

### C. Self-signed install (no developer mode, needs admin once)

```powershell
# 1. Generate a dev cert and import it into LocalMachine\TrustedPeople.
#    The trust import needs an elevated PowerShell.
pwsh -File scripts\dev-cert.ps1

# 2. Build a signed MSIX with that cert (paste the thumbprint the
#    previous step printed).
pwsh -File scripts\build-msix.ps1 -CertThumbprint <thumb>

# 3. Now the MSIX double-click installs cleanly without developer mode.
Add-AppxPackage -Path .\dist\npurun-<version>-aarch64-windows.msix
```

The `.msix` only contains `npurun.exe` plus tile icons — it does **not**
bundle the QAIRT runtime (Qualcomm doesn't allow redistribution). You still
need [the QAIRT setup](#qairt-runtime) below before any inference command
will work.

## 2. ZIP (portable)

For users who don't want a registered AppX install, or for CI:

1. Download `npurun-<version>-aarch64-windows.zip` from the [latest
   release](https://github.com/bpbonker/npurun/releases).
2. Extract anywhere. The zip contains a single `npurun.exe`.
3. Add the extract directory to PATH, or invoke `npurun.exe` by full path.

The same QAIRT setup applies.

## 3. Build from source

```powershell
git clone https://github.com/bpbonker/npurun.git
cd npurun

# Validate the QAIRT install before the first build.
pwsh -File scripts\setup-qnn.ps1

# Build through dev-shell so MSVC ARM64 + LLVM + QAIRT are on PATH.
scripts\dev-shell.bat cargo build --release -p npurun-cli

# Binary at target\release\npurun.exe
```

You'll need the full toolchain set listed in the [README prerequisites
section](https://github.com/bpbonker/npurun#prerequisites): MSVC v143 ARM64
+ Win11 SDK 26100, LLVM/clang, Rust stable + the
`aarch64-pc-windows-msvc` target, and the QAIRT SDK 2.44+.

## QAIRT runtime

The QAIRT SDK is **not redistributable** — install it manually from the
[Qualcomm developer portal](https://www.qualcomm.com/developer). Once
installed:

```powershell
# Point npurun at it. Persist this so future shells inherit it.
[Environment]::SetEnvironmentVariable("QNN_SDK_ROOT", "C:\path\to\qairt\2.45.0", "User")

# Validate the install (checks SDK layout + Hexagon NPU device presence).
pwsh -File scripts\setup-qnn.ps1
```

`npurun.exe` reads `QNN_SDK_ROOT` at startup and prepends the QAIRT
`bin\aarch64-windows-msvc` and `lib\aarch64-windows-msvc` directories to its
own DLL search path so `Genie.dll` / `QnnHtp.dll` / the Hexagon stub
libraries load. If you launch `npurun.exe` from a shell that doesn't have
`QNN_SDK_ROOT` set, you'll see Windows error `0xC0000135`
(STATUS_DLL_NOT_FOUND) at process start.

## Verifying the install

```powershell
npurun version
```

Should print three lines (semver, libGenie, QAIRT path). If any of these
fails or shows `unknown`, see
[`troubleshooting.md`](troubleshooting.md).

```text
npurun       0.1.0-rc.2
libGenie     1.17.0
QAIRT SDK    2.45.0  (C:\path\to\qairt\2.45.0)
```

Once `npurun version` is clean, head to [`usage.md`](usage.md) for the
`pull → run → serve` walkthrough.

## Uninstall

```powershell
# MSIX install
Get-AppxPackage *npurun* | Remove-AppxPackage

# Cached models live separately; delete if you want them gone too.
Remove-Item -Recurse $env:LOCALAPPDATA\npurun\models
```

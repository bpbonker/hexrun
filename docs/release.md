# Release runbook

Step-by-step for cutting a hexrun release. Aims to be the literal
copy-paste flow rather than aspirational.

## Pre-flight (once per machine)

You need:
- Rust stable + `aarch64-pc-windows-msvc` target.
- MSVC v143 ARM64/ARM64EC + Win11 SDK 26100.
- LLVM/clang.
- QAIRT 2.45 with `QNN_SDK_ROOT` set.
- `gh` CLI authenticated against the bpbonker account.

Validate:
```powershell
pwsh -File scripts\setup-qnn.ps1
gh auth status
```

## Cutting a release

The local flow lives in `scripts\package-release.ps1` (zip) and
`scripts\build-msix.ps1` (MSIX). The eventual cloud flow is in
`.github\workflows\release.yml`, currently disabled until a
self-hosted ARM64 runner with QAIRT is enrolled.

### 1. Land the code, decide on the version

Conventions:
- `0.1.0-rc.N` — release candidates. Pre-release on GitHub.
- `0.1.0` — first stable.
- `0.1.X` — patch releases off `0.1`.

Update `Cargo.toml`'s `[workspace.package].version` and commit.

### 2. Build the artifacts

```powershell
# Bumps + commits the workspace version, builds release zip + MSIX,
# emits sha256 sidecars under dist/.
$v = "0.1.0-rc.2"  # or whatever
pwsh -File scripts\package-release.ps1 -Version $v
pwsh -File scripts\build-msix.ps1 -Version $v
```

Outputs:
- `dist\hexrun-$v-aarch64-windows.zip` and `.zip.sha256`
- `dist\hexrun-$v-aarch64-windows.msix` and `.msix.sha256`

### 3. Smoke-test the zip

On a clean PowerShell session (so no env state leaks in):

```powershell
$work = mkdir "$env:TEMP\hexrun-smoke-$v" -Force
Expand-Archive "dist\hexrun-$v-aarch64-windows.zip" -DestinationPath $work
$work\hexrun.exe version
$env:HEXRUN_MODELS_DIR = "$env:LOCALAPPDATA\hexrun\models"
$work\hexrun.exe show phi-3.5-mini   # if you have it cached
```

If `version` errors with a DLL load failure, your `QNN_SDK_ROOT`
isn't set in this terminal — that's the same friction users will
hit, so document it in the release notes (already covered in
`INSTALL.md` inside the zip).

### 4. Tag, push, and create the GitHub release

```powershell
git tag -a v$v -m "v$v"
git push origin v$v

gh release create v$v `
    "dist\hexrun-$v-aarch64-windows.zip" `
    "dist\hexrun-$v-aarch64-windows.zip.sha256" `
    "dist\hexrun-$v-aarch64-windows.msix" `
    "dist\hexrun-$v-aarch64-windows.msix.sha256" `
    --title "v$v" `
    --notes-file "dist\RELEASE_NOTES.md" `
    --prerelease   # drop for stable
```

(Author the release notes by hand as `dist\RELEASE_NOTES.md` — see
the `v0.1.0-rc.1` notes for the structure: headline numbers, what's
in, quick start, artifacts table with sha256s, caveats, and a
license/no-liability section.)

### 5. Update the winget manifest

For each new release, copy the previous version's manifest dir,
bump the version + hash, and validate.

```powershell
# Copy the previous version's manifests as a starting point:
$prev = "0.1.0-rc.1"
Copy-Item -Recurse `
    "manifests\b\bpbonker\hexrun\$prev" `
    "manifests\b\bpbonker\hexrun\$v"

# Edit the three .yaml files in the new dir:
#   - bpbonker.hexrun.yaml             — bump PackageVersion
#   - bpbonker.hexrun.locale.en-US.yaml — bump PackageVersion + ReleaseNotesUrl
#   - bpbonker.hexrun.installer.yaml   — bump PackageVersion + InstallerUrl + InstallerSha256
#
# The sha256 is whatever you wrote into dist\hexrun-$v-aarch64-windows.zip.sha256,
# uppercased.

winget validate --manifest "manifests\b\bpbonker\hexrun\$v"
```

Commit the manifest dir alongside any other release-prep changes.

### Submitting to the public winget catalog (optional, requires signed MSIX)

`winget install bpbonker.hexrun` works against Microsoft's public
catalog only after the manifest is merged into
`microsoft/winget-pkgs`. That requires:

1. A code-signed installer. The unsigned `.msix` we ship today
   doesn't qualify; a `.zip` with a portable nested binary does
   work but the catalog reviewers prefer signed installers.
2. A pull request to `microsoft/winget-pkgs` with the manifest dir
   placed at `manifests/b/bpbonker/hexrun/$v/`.
3. Passing the catalog's automated validation (it will install the
   package on a sandbox VM and run a sanity check).

Until then, users can install directly from this repo:

```powershell
gh repo clone bpbonker/hexrun
winget install --manifest hexrun\manifests\b\bpbonker\hexrun\0.1.0-rc.1
```

## Code signing

### Dev (self-signed)

For local-only signed-MSIX testing on the dev laptop:

```powershell
pwsh -File scripts\dev-cert.ps1
# prints a thumbprint; pass it to build-msix.ps1
pwsh -File scripts\build-msix.ps1 -CertThumbprint <thumb>
```

The script imports the cert into both `CurrentUser\My` and
`LocalMachine\TrustedPeople` (the second needs admin) so MSIX
install accepts it without developer mode.

### Production

A real cert from a CA (or an EV cert) is the requirement for public
distribution and for catalog submission. Three viable paths:

- **Azure Trusted Signing** ($10/month, no hardware token, modern).
  Microsoft's recommended path for new ISVs in 2025+.
- **DigiCert / Sectigo standard code-signing cert** (~$200-400/year,
  software cert, no hardware token).
- **EV cert** (~$300-500/year + USB token). Trusted immediately by
  SmartScreen; standard code-signing certs warm up over time.

Once a cert is provisioned, expose its thumbprint to CI as the
`MSIX_CERT_THUMBPRINT` secret and the release workflow signs MSIXes
automatically.

## CI matrix

Cloud CI (`.github\workflows\ci.yml`):

| Job | Runner | What it does |
|---|---|---|
| `fmt-clippy` | `windows-latest` (x64) | `cargo fmt --check`, `cargo clippy` on the non-QNN crates |
| `build-default` | `windows-latest` (x64) | `cargo build` on the non-QNN crates |
| `test-default` | `windows-latest` (x64) | `cargo test` on the non-QNN crates |
| `python-lint` | `windows-latest` | `ruff check` on `python/hex-convert/` |
| `python-test` | `windows-latest` | `pytest` on `python/hex-convert/` |
| `winget-validate` | `windows-latest` | `winget validate` on every published manifest dir |

QAIRT is not in cloud CI (Qualcomm's SDK is non-redistributable);
the `qnn` and `qnn-sys` crates are excluded from the cloud builds.

Self-hosted CI (`build-arm64-with-qnn` in `ci.yml`,
`build-and-release` in `release.yml`): both gated `if: false`.
Enable once a Snapdragon X Elite laptop is enrolled as a self-hosted
runner with `QNN_SDK_ROOT` set as a secret.

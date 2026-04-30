# scripts/package-release.ps1
#
# Build the release binary, stage the artifacts a user actually needs to
# run npurun, and emit a versioned zip + sha256.
#
# What's in the zip:
#   - npurun.exe                    (the binary)
#   - README.md, CHANGELOG.md       (entry points)
#   - LICENSE-MIT, LICENSE-APACHE-2.0
#   - scripts/setup-qnn.ps1         (validates the user's QAIRT install)
#   - scripts/energy-bench.ps1      (optional power-measurement script)
#   - docs/                         (handoff, benchmarks, troubleshooting, etc.)
#   - INSTALL.md                    (generated; gives the 60-second runbook)
#
# What's NOT in the zip:
#   - Genie.dll, QnnHtp.dll, Hexagon stubs — these come from the user's
#     QAIRT SDK install. Qualcomm does not allow redistribution.
#
# Usage:
#   pwsh -File scripts\package-release.ps1
#   pwsh -File scripts\package-release.ps1 -Version 0.1.0-rc.1
#   pwsh -File scripts\package-release.ps1 -SkipBuild  # if you already built

[CmdletBinding()]
param(
    [string]$Version = "",
    [switch]$SkipBuild,
    [string]$OutDir = "dist"
)

$ErrorActionPreference = "Stop"

# --- resolve version ---
if ([string]::IsNullOrEmpty($Version)) {
    $cargoToml = Get-Content "Cargo.toml" -Raw
    if ($cargoToml -match 'version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        throw "could not parse workspace version from Cargo.toml"
    }
}
Write-Host "==  npurun release packager  ==" -ForegroundColor Cyan
Write-Host "  version:   $Version"

# --- build ---
if (-not $SkipBuild) {
    Write-Host ""
    Write-Host "[1/4] cargo build --release -p npurun-cli" -ForegroundColor DarkGray
    if (-not (Test-Path "scripts\dev-shell.bat")) {
        throw "scripts\dev-shell.bat not found; run from the repo root"
    }
    & cmd.exe /c "scripts\dev-shell.bat cargo build --release -p npurun-cli"
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed (exit $LASTEXITCODE)"
    }
} else {
    Write-Host ""
    Write-Host "[1/4] skipping build (-SkipBuild)" -ForegroundColor DarkGray
}

$binary = "target\release\npurun.exe"
if (-not (Test-Path $binary)) {
    throw "expected $binary after build, but it's not there"
}

# --- stage ---
$stem = "npurun-$Version-aarch64-windows"
$staging = Join-Path $OutDir $stem
Write-Host ""
Write-Host "[2/4] staging into $staging" -ForegroundColor DarkGray
if (Test-Path $staging) {
    Remove-Item $staging -Recurse -Force
}
New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $staging "scripts") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $staging "docs") -Force | Out-Null

Copy-Item $binary -Destination (Join-Path $staging "npurun.exe")
Copy-Item "README.md" -Destination $staging
Copy-Item "CHANGELOG.md" -Destination $staging
Copy-Item "LICENSE-MIT" -Destination $staging
Copy-Item "LICENSE-APACHE" -Destination $staging
Copy-Item "scripts\setup-qnn.ps1" -Destination (Join-Path $staging "scripts")
Copy-Item "scripts\energy-bench.ps1" -Destination (Join-Path $staging "scripts")
foreach ($doc in @("handoff.md","benchmarks.md","troubleshooting.md","compatibility.md","architecture.md","roadmap.md","findings.md","paper.md")) {
    $src = "docs\$doc"
    if (Test-Path $src) {
        Copy-Item $src -Destination (Join-Path $staging "docs")
    }
}

# --- generate INSTALL.md ---
$installMd = @"
# npurun $Version — Windows ARM64

## What this is

A standalone build of ``npurun.exe`` for Snapdragon X Elite Windows on
ARM64. NPU-first local LLM runtime — see ``README.md`` for the full
project description.

## What you still need

npurun depends on Qualcomm's QAIRT SDK at runtime. The SDK is **not
redistributable**, so this archive does not include it. You'll need:

1. **QAIRT SDK 2.44 or 2.45** — download from the
   [Qualcomm developer portal](https://www.qualcomm.com/developer)
   (free account; they call it "Qualcomm AI Engine Direct").
   Extract to a path like ``C:\Qualcomm\AIStack\QAIRT\2.45.0.250130``.
2. The ``QNN_SDK_ROOT`` environment variable set to that path.

Validate the install:

````powershell
``$env:QNN_SDK_ROOT = "C:\Qualcomm\AIStack\QAIRT\2.45.0.250130"
pwsh -File scripts\setup-qnn.ps1
````

## 60-second runbook

````powershell
# 1. Tell npurun where to keep models (any folder you have write access to).
``$env:NPURUN_MODELS_DIR = "``$env:LOCALAPPDATA\npurun\models"

# 2. Pull a model.
.\npurun.exe pull phi-3.5-mini

# 3. Run a one-shot generation.
.\npurun.exe run phi-3.5-mini "Tell me a one-line joke about Snapdragon laptops."

# 4. Or run the OpenAI/Ollama-compatible HTTP server.
.\npurun.exe serve --model phi-3.5-mini
# Server is at http://localhost:11435 — point Open WebUI at it.
````

## Verifying you're actually on the NPU

Open Task Manager → Performance → NPU. While ``npurun run`` is generating,
the NPU column should show 19–30% utilization. If it stays at 0% but
text is being produced, you're on a CPU fallback — file an issue.

## Support

If npurun saved you time, you can buy me a coffee:
<https://buymeacoffee.com/bpbprofessional>

Bug reports / feature requests / contributions: see the GitHub repo.
"@
Set-Content -Path (Join-Path $staging "INSTALL.md") -Value $installMd -Encoding UTF8

# --- zip ---
$zipPath = Join-Path $OutDir "$stem.zip"
Write-Host ""
Write-Host "[3/4] zipping to $zipPath" -ForegroundColor DarkGray
if (Test-Path $zipPath) {
    Remove-Item $zipPath -Force
}
Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $zipPath -CompressionLevel Optimal

# --- sha256 ---
Write-Host ""
Write-Host "[4/4] sha256" -ForegroundColor DarkGray
$hash = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
$shaFile = "$zipPath.sha256"
"$hash  $stem.zip" | Set-Content -Path $shaFile -Encoding ASCII

# --- summary ---
$zipBytes = (Get-Item $zipPath).Length
$zipMb = [math]::Round($zipBytes / 1MB, 2)
Write-Host ""
Write-Host "==  release ready  ==" -ForegroundColor Cyan
Write-Host ("  artifact:  {0}  ({1} MB)" -f $zipPath, $zipMb)
Write-Host ("  sha256:    {0}" -f $hash)
Write-Host ("  checksum:  {0}" -f $shaFile)
Write-Host ""
Write-Host "next steps:"
Write-Host "  - smoke-test on a clean machine (set QNN_SDK_ROOT, follow INSTALL.md)"
Write-Host "  - tag the release: git tag v$Version && git push origin v$Version"
Write-Host "  - upload $zipPath + $shaFile as the GitHub release assets"
Write-Host ""

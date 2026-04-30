# scripts/build-msix.ps1
#
# Build an MSIX package for npurun on Windows ARM64.
#
# This produces an .msix that wraps npurun.exe as a Win32 desktop app
# (Windows.FullTrustApplication entry point, runFullTrust capability).
# Without code-signing, the resulting .msix can only be installed by
# users who have enabled developer mode (Settings > For developers).
# Once we have a real signing cert (Phase 6), pass -CertThumbprint to
# emit a signed .msix that installs by double-click.
#
# Generates placeholder Assets/*.png icons on first run. Replace them
# in installer/Assets/ with real artwork before shipping a public build.
#
# Usage:
#   pwsh -File scripts\build-msix.ps1
#   pwsh -File scripts\build-msix.ps1 -Version 0.1.0-rc.1
#   pwsh -File scripts\build-msix.ps1 -CertThumbprint <cert sha1>

[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$CertThumbprint = "",
    [string]$OutDir = "dist",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

# --- locate makeappx + signtool ---
function Find-WindowsKitTool($toolName) {
    $roots = @(
        "C:\Program Files (x86)\Windows Kits\10\bin",
        "C:\Program Files\Windows Kits\10\bin"
    )
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        # Prefer the newest SDK and arm64 toolchain when available.
        $sdkVersions = Get-ChildItem $root -Directory | Where-Object { $_.Name -match '^10\.' } | Sort-Object Name -Descending
        foreach ($sdk in $sdkVersions) {
            foreach ($arch in @("arm64", "x64")) {
                $candidate = Join-Path $sdk.FullName "$arch\$toolName"
                if (Test-Path $candidate) {
                    return $candidate
                }
            }
        }
    }
    return $null
}

$makeappx = Find-WindowsKitTool "makeappx.exe"
if (-not $makeappx) {
    throw "makeappx.exe not found. Install the Windows 11 SDK (26100) via Visual Studio Installer."
}
Write-Host "  makeappx:  $makeappx" -ForegroundColor DarkGray

$signtool = $null
if ($CertThumbprint) {
    $signtool = Find-WindowsKitTool "signtool.exe"
    if (-not $signtool) {
        throw "signtool.exe not found but -CertThumbprint was supplied."
    }
    Write-Host "  signtool:  $signtool" -ForegroundColor DarkGray
}

# --- resolve version ---
if ([string]::IsNullOrEmpty($Version)) {
    $cargoToml = Get-Content "Cargo.toml" -Raw
    if ($cargoToml -match 'version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        throw "could not parse workspace version from Cargo.toml"
    }
}
# MSIX version must be 4-part numeric. Strip pre-release tags.
$msixVersion = ($Version -replace '-.*$', '')
if (($msixVersion.Split('.').Count) -eq 3) {
    $msixVersion += ".0"
}
Write-Host "==  npurun MSIX builder  ==" -ForegroundColor Cyan
Write-Host "  semver:    $Version"
Write-Host "  msix ver:  $msixVersion"

# --- build ---
if (-not $SkipBuild) {
    Write-Host ""
    Write-Host "[1/5] cargo build --release -p npurun-cli" -ForegroundColor DarkGray
    & cmd.exe /c "scripts\dev-shell.bat cargo build --release -p npurun-cli"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
}
$binary = "target\release\npurun.exe"
if (-not (Test-Path $binary)) { throw "expected $binary after build, but it's not there" }

# --- placeholder icons ---
# MSIX requires the asset PNGs referenced in AppxManifest.xml. We
# generate solid-color squares with a single letter so a developer
# build is buildable end-to-end. Replace these with real artwork
# before a public release.
$assetsDir = "installer\Assets"
if (-not (Test-Path $assetsDir)) {
    New-Item -ItemType Directory -Path $assetsDir | Out-Null
}
$assetSpecs = @(
    @{ Name = "StoreLogo.png";          W = 50;  H = 50;  Letter = "h" },
    @{ Name = "Square44x44Logo.png";    W = 44;  H = 44;  Letter = "h" },
    @{ Name = "Square150x150Logo.png";  W = 150; H = 150; Letter = "npurun" },
    @{ Name = "Wide310x150Logo.png";    W = 310; H = 150; Letter = "npurun" }
)
Add-Type -AssemblyName System.Drawing
foreach ($spec in $assetSpecs) {
    $path = Join-Path $assetsDir $spec.Name
    if (Test-Path $path) { continue }
    $bmp = New-Object System.Drawing.Bitmap($spec.W, $spec.H)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $bg = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(31, 31, 31))
    $g.FillRectangle($bg, 0, 0, $spec.W, $spec.H)
    $fontSize = [Math]::Min($spec.W, $spec.H) * 0.45
    $font = New-Object System.Drawing.Font("Segoe UI", $fontSize, [System.Drawing.FontStyle]::Bold)
    $fg = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 215, 0))
    $sf = New-Object System.Drawing.StringFormat
    $sf.Alignment = [System.Drawing.StringAlignment]::Center
    $sf.LineAlignment = [System.Drawing.StringAlignment]::Center
    $rect = New-Object System.Drawing.RectangleF(0, 0, $spec.W, $spec.H)
    $g.DrawString($spec.Letter, $font, $fg, $rect, $sf)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "  generated $path ($($spec.W)x$($spec.H))" -ForegroundColor DarkGray
}

# --- stage ---
$stem = "npurun-$Version-aarch64-windows"
$staging = Join-Path $OutDir "$stem-msix-staging"
Write-Host ""
Write-Host "[2/5] staging into $staging" -ForegroundColor DarkGray
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $staging "Assets") -Force | Out-Null

# Manifest: substitute the version, then write into staging.
# Use UTF-8 *without* BOM — makeappx rejects a BOM ahead of the
# `<?xml ... ?>` declaration with "Incorrect xml declaration syntax".
$manifest = Get-Content "installer\AppxManifest.xml" -Raw
# Use case-sensitive replace so we don't clobber the lowercase
# `version="1.0"` in the `<?xml ... ?>` declaration.
$manifest = $manifest -creplace 'Version="[\d.]+"', "Version=`"$msixVersion`""
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText((Join-Path (Resolve-Path $staging) "AppxManifest.xml"), $manifest, $utf8NoBom)

Copy-Item $binary -Destination (Join-Path $staging "npurun.exe")
Copy-Item (Join-Path $assetsDir "*.png") -Destination (Join-Path $staging "Assets")

# --- pack ---
if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir | Out-Null }
$msixPath = Join-Path $OutDir "$stem.msix"
Write-Host ""
Write-Host "[3/5] makeappx pack -> $msixPath" -ForegroundColor DarkGray
if (Test-Path $msixPath) { Remove-Item $msixPath -Force }
& $makeappx pack /d $staging /p $msixPath /o
if ($LASTEXITCODE -ne 0) { throw "makeappx pack failed (exit $LASTEXITCODE)" }

# --- sign (optional) ---
if ($CertThumbprint) {
    Write-Host ""
    Write-Host "[4/5] signtool sign $msixPath" -ForegroundColor DarkGray
    & $signtool sign /sha1 $CertThumbprint /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 $msixPath
    if ($LASTEXITCODE -ne 0) { throw "signtool sign failed (exit $LASTEXITCODE)" }
} else {
    Write-Host ""
    Write-Host "[4/5] skipping sign (no -CertThumbprint)" -ForegroundColor DarkGray
}

# --- sha256 ---
Write-Host ""
Write-Host "[5/5] sha256" -ForegroundColor DarkGray
$hash = (Get-FileHash -Algorithm SHA256 $msixPath).Hash.ToLower()
$shaFile = "$msixPath.sha256"
"$hash  $stem.msix" | Set-Content -Path $shaFile -Encoding ASCII

# --- summary ---
$bytes = (Get-Item $msixPath).Length
$mb = [math]::Round($bytes / 1MB, 2)
Write-Host ""
Write-Host "==  MSIX ready  ==" -ForegroundColor Cyan
Write-Host ("  artifact:  {0}  ({1} MB)" -f $msixPath, $mb)
Write-Host ("  sha256:    {0}" -f $hash)
Write-Host ("  signed:    {0}" -f $(if ($CertThumbprint) { "yes" } else { "no - only installable in Windows developer mode" }))
Write-Host ""
if (-not $CertThumbprint) {
    Write-Host "to install this unsigned MSIX:" -ForegroundColor Yellow
    Write-Host "  1. Settings > Privacy & Security > For developers > Developer Mode = ON"
    Write-Host "  2. Add-AppxPackage -Path $msixPath"
    Write-Host ""
}

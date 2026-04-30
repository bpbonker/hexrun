# scripts/dev-shell.ps1
#
# Drop into a PowerShell session with everything wired up to build npurun:
# - cargo / rustc on PATH (from %USERPROFILE%\.cargo\bin)
# - MSVC dev environment (link.exe, lib.exe, cl.exe, INCLUDE, LIB, LIBPATH)
#   for the ARM64 host targeting ARM64
# - QNN_SDK_ROOT exported if installed
#
# Usage:
#   pwsh -NoExit -File scripts\dev-shell.ps1
# Or:
#   pwsh -File scripts\dev-shell.ps1 -Command "cargo build --release"

[CmdletBinding()]
param(
    [string]$VsInstallPath,
    [ValidateSet("arm64", "x64_arm64", "x64", "arm64_x64")]
    [string]$Arch = "arm64",
    [string]$Command
)

$ErrorActionPreference = "Stop"

function Write-Section($Text) { Write-Host "== $Text ==" -ForegroundColor Cyan }

# Cargo on PATH
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
    if (-not ($env:PATH -split ";" | Where-Object { $_ -ieq $cargoBin })) {
        $env:PATH = "$cargoBin;$env:PATH"
    }
    Write-Section "Rust"
    & cargo --version
    & rustc --version
} else {
    Write-Warning "Rust toolchain not found at $cargoBin. Run: winget install Rustlang.Rustup"
}

# Locate VS install
if (-not $VsInstallPath) {
    $vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $VsInstallPath = & $vswhere -latest -products '*' -property installationPath
    }
}
if (-not $VsInstallPath -or -not (Test-Path $VsInstallPath)) {
    Write-Warning "Visual Studio install not found. C++ build will fail."
} else {
    Write-Section "VS install"
    Write-Host $VsInstallPath
    $vcvars = Join-Path $VsInstallPath "VC\Auxiliary\Build\vcvarsall.bat"
    if (-not (Test-Path $vcvars)) {
        Write-Warning "vcvarsall.bat not found at $vcvars"
    } else {
        Write-Section "Loading MSVC env (arch=$Arch)"
        $cmd = "`"$vcvars`" $Arch && set"
        $envOut = cmd /c $cmd
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "vcvarsall.bat exited with code $LASTEXITCODE for arch=$Arch."
            Write-Warning "If you don't have the ARM64/ARM64EC C++ build tools installed,"
            Write-Warning "open the VS Installer and add: 'MSVC v143 - VS 2022 C++ ARM64/ARM64EC build tools (Latest)'."
        } else {
            foreach ($line in $envOut) {
                if ($line -match '^([^=]+)=(.*)$') {
                    [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
                }
            }
            Write-Host "  link.exe  -> $(Get-Command link.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)"
            Write-Host "  cl.exe    -> $(Get-Command cl.exe   -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)"
        }
    }
}

# QNN
Write-Section "QNN"
if ($env:QNN_SDK_ROOT) {
    Write-Host "  QNN_SDK_ROOT = $env:QNN_SDK_ROOT"
} else {
    Write-Warning "QNN_SDK_ROOT not set. qnn-sys will emit stub bindings only. Run scripts\setup-qnn.ps1."
}

if ($Command) {
    Write-Section "Running: $Command"
    Invoke-Expression $Command
}

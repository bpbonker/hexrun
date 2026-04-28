# scripts/setup-qnn.ps1
#
# Validates a Qualcomm AI Engine Direct (QNN) SDK install on Windows ARM64
# and confirms the Hexagon NPU is present. Does not download the SDK
# (license forbids redistribution) — the user must install it from the
# Qualcomm developer portal first.
#
# Usage:
#   pwsh -File scripts\setup-qnn.ps1
#   pwsh -File scripts\setup-qnn.ps1 -SdkRoot "C:\Qualcomm\AIStack\QAIRT\2.44.0"

[CmdletBinding()]
param(
    [string]$SdkRoot = $env:QNN_SDK_ROOT,
    [string]$MinSdkVersion = "2.40.0"
)

$ErrorActionPreference = "Stop"

function Write-Section($Text) {
    Write-Host ""
    Write-Host "== $Text ==" -ForegroundColor Cyan
}

function Write-Ok($Text)   { Write-Host "  [ok]    $Text" -ForegroundColor Green }
function Write-Warn2($T)   { Write-Host "  [warn]  $T"   -ForegroundColor Yellow }
function Write-Fail($Text) { Write-Host "  [fail]  $Text" -ForegroundColor Red }

$problems = @()

Write-Section "Hexagon NPU device"
$npu = Get-PnpDevice -ErrorAction SilentlyContinue |
    Where-Object { $_.FriendlyName -match 'Hexagon|NPU' -and $_.Manufacturer -match 'Qualcomm' }
if ($null -ne $npu) {
    foreach ($d in $npu) {
        $status = if ($d.Status -eq 'OK') { 'Write-Ok' } else { 'Write-Warn2' }
        & $status "$($d.FriendlyName)  [$($d.Status)]"
    }
} else {
    Write-Fail "No Qualcomm Hexagon NPU device found. This script must run on a Snapdragon X Elite / X Plus PC."
    $problems += "no NPU device"
}

Write-Section "QNN SDK location"
if ([string]::IsNullOrWhiteSpace($SdkRoot)) {
    Write-Fail "QNN_SDK_ROOT is not set and no -SdkRoot argument was given."
    Write-Host "        Install the QNN SDK from https://www.qualcomm.com/developer (Qualcomm AI Engine Direct)" -ForegroundColor DarkGray
    Write-Host "        and either set QNN_SDK_ROOT or pass -SdkRoot to this script." -ForegroundColor DarkGray
    $problems += "QNN_SDK_ROOT unset"
} elseif (-not (Test-Path $SdkRoot)) {
    Write-Fail "QNN_SDK_ROOT points to a path that does not exist: $SdkRoot"
    $problems += "QNN_SDK_ROOT path missing"
} else {
    Write-Ok "SdkRoot = $SdkRoot"

    Write-Section "Required headers"
    $headers = @(
        "include\QNN\QnnCommon.h",
        "include\QNN\QnnInterface.h",
        "include\QNN\QnnBackend.h",
        "include\QNN\QnnContext.h",
        "include\QNN\QnnGraph.h",
        "include\QNN\QnnTensor.h"
    )
    foreach ($h in $headers) {
        $p = Join-Path $SdkRoot $h
        if (Test-Path $p) { Write-Ok $h } else { Write-Fail "missing $h"; $problems += "missing header $h" }
    }

    Write-Section "Required runtime DLLs"
    $libCandidates = @(
        "lib\aarch64-windows-msvc",
        "lib\arm64x-windows-msvc",
        "lib\x86_64-windows-msvc"
    )
    $libDir = $null
    foreach ($cand in $libCandidates) {
        $p = Join-Path $SdkRoot $cand
        if (Test-Path $p) { $libDir = $p; break }
    }
    if ($null -eq $libDir) {
        Write-Fail "No expected lib directory under $SdkRoot\lib (looked for: $($libCandidates -join ', '))"
        $problems += "no lib dir"
    } else {
        Write-Ok "libDir = $libDir"
        $dlls = @("QnnSystem.dll", "QnnHtp.dll", "QnnCpu.dll")
        foreach ($d in $dlls) {
            $p = Join-Path $libDir $d
            if (Test-Path $p) {
                $info = (Get-Item $p).VersionInfo
                Write-Ok ("{0}  v{1}" -f $d, $info.FileVersion)
            } else {
                Write-Warn2 "missing $d (in $libDir)"
            }
        }
    }

    Write-Section "Reported SDK version"
    $verFile = Join-Path $SdkRoot "sdk.yaml"
    if (Test-Path $verFile) {
        $content = Get-Content $verFile -Raw
        if ($content -match 'version:\s*([0-9]+\.[0-9]+\.[0-9]+)') {
            $sdkVer = $matches[1]
            Write-Ok "SDK version $sdkVer"
            if ([version]$sdkVer -lt [version]$MinSdkVersion) {
                Write-Warn2 "SDK $sdkVer is older than the recommended minimum $MinSdkVersion"
            }
        } else {
            Write-Warn2 "could not parse version from sdk.yaml"
        }
    } else {
        Write-Warn2 "sdk.yaml not found under $SdkRoot — version check skipped"
    }
}

Write-Section "Persisted environment"
$persist = [Environment]::GetEnvironmentVariable("QNN_SDK_ROOT", "User")
if ([string]::IsNullOrWhiteSpace($persist)) {
    Write-Warn2 "QNN_SDK_ROOT is not set in the User environment (only the current process)."
    Write-Host "        To persist:  setx QNN_SDK_ROOT `"$SdkRoot`"" -ForegroundColor DarkGray
} else {
    Write-Ok "QNN_SDK_ROOT (User) = $persist"
}

Write-Host ""
if ($problems.Count -eq 0) {
    Write-Host "All checks passed. You can now build hexrun:  cargo build --release" -ForegroundColor Green
    exit 0
} else {
    Write-Host ("Setup incomplete. Issues: " + ($problems -join "; ")) -ForegroundColor Red
    exit 1
}

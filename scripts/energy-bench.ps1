# scripts/energy-bench.ps1
#
# Measures NPU inference energy by sampling battery DischargeRate
# (milliwatts) before, during, and after a `hexrun bench` run.
#
# Output: idle power baseline, busy power average, delta watts attributable
# to inference, energy-per-token in joules.
#
# Requirements:
#   - Laptop must be on battery (unplugged) — DischargeRate is only valid
#     when discharging. The script bails if BatteryStatus reports plugged in.
#   - hexrun.exe in target/release (build first).
#   - HEXRUN_MODELS_DIR set or %LOCALAPPDATA%\hexrun\models populated.
#
# Usage:
#   pwsh -File scripts\energy-bench.ps1 -Model phi-3.5-mini
#   pwsh -File scripts\energy-bench.ps1 -Model phi-3.5-mini -BaselineSeconds 20

[CmdletBinding()]
param(
    [string]$Model = "phi-3.5-mini",
    [int]$BaselineSeconds = 30,
    [int]$SampleIntervalMs = 500,
    [string]$Hexrun = (Join-Path (Get-Location) "target\release\hexrun.exe"),
    [string]$ModelsDir = $env:HEXRUN_MODELS_DIR
)

if (-not [string]::IsNullOrEmpty($ModelsDir)) {
    $env:HEXRUN_MODELS_DIR = $ModelsDir
    Write-Host "  HEXRUN_MODELS_DIR set to $ModelsDir"
} elseif ([string]::IsNullOrEmpty($env:HEXRUN_MODELS_DIR)) {
    # Fall back to the well-known dev location.
    $candidate = "C:\AAA\Personal\AI\models"
    if (Test-Path $candidate) {
        $env:HEXRUN_MODELS_DIR = $candidate
        Write-Host "  HEXRUN_MODELS_DIR auto-set to $candidate"
    }
}

$ErrorActionPreference = "Stop"

function Get-DischargeRateMilliwatts {
    # Returns milliwatts being drawn from the battery, or $null if not discharging.
    $b = Get-CimInstance -Namespace root\wmi -ClassName BatteryStatus -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $b) { return $null }
    if ($b.Discharging -ne $true) { return $null }
    return [int]$b.DischargeRate
}

function Read-PowerSamples($Seconds, $IntervalMs) {
    $end = (Get-Date).AddSeconds($Seconds)
    $samples = [System.Collections.Generic.List[double]]::new()
    while ((Get-Date) -lt $end) {
        $mw = Get-DischargeRateMilliwatts
        if ($null -ne $mw) { [void]$samples.Add([double]$mw) }
        Start-Sleep -Milliseconds $IntervalMs
    }
    return ,$samples
}

function Mean($collection) {
    if ($collection.Count -eq 0) { return [double]::NaN }
    return ($collection | Measure-Object -Average).Average
}

if (-not (Test-Path $Hexrun)) {
    throw "hexrun binary not found at $Hexrun. Build first: cargo build --release -p hexrun-cli"
}

# Make Genie.dll, QnnHtp.dll and the Hexagon stubs reachable at process
# startup. Without this the spawned hexrun.exe fails with STATUS_DLL_NOT_FOUND
# before main runs.
if ($env:QNN_SDK_ROOT) {
    $bin = Join-Path $env:QNN_SDK_ROOT "bin\aarch64-windows-msvc"
    $lib = Join-Path $env:QNN_SDK_ROOT "lib\aarch64-windows-msvc"
    $adsp = Join-Path $env:QNN_SDK_ROOT "lib\hexagon-v73\unsigned"
    if ((Test-Path $bin) -and (Test-Path $lib) -and (Test-Path $adsp)) {
        $env:Path = "$bin;$lib;$env:Path"
        if (-not $env:ADSP_LIBRARY_PATH) {
            $env:ADSP_LIBRARY_PATH = $adsp
        }
        Write-Host "  QAIRT paths added to PATH and ADSP_LIBRARY_PATH"
    } else {
        Write-Warning "QNN_SDK_ROOT is set but expected subdirs not found; hexrun may fail to load DLLs."
    }
} else {
    Write-Warning "QNN_SDK_ROOT is not set; hexrun.exe will likely fail at startup with a DLL load error."
}

$initial = Get-DischargeRateMilliwatts
if ($null -eq $initial) {
    throw "battery is not discharging (probably plugged in). Unplug the laptop and re-run; the script needs DischargeRate to compute energy."
}

Write-Host ""
Write-Host "==  hexrun energy benchmark  ==" -ForegroundColor Cyan
Write-Host "  hexrun:        $Hexrun"
Write-Host "  model:         $Model"
Write-Host "  baseline:      $BaselineSeconds seconds"
Write-Host "  sample rate:   every $SampleIntervalMs ms"
Write-Host ""

# --- baseline ---
Write-Host "[1/3] sampling idle baseline..." -ForegroundColor DarkGray
$idle = Read-PowerSamples $BaselineSeconds $SampleIntervalMs
$idle_mean_mw = Mean $idle
Write-Host ("       idle mean: {0:N0} mW ({1:N2} W) over {2} samples" -f $idle_mean_mw, ($idle_mean_mw / 1000), $idle.Count)

# --- run hexrun bench in a background process and sample while it runs ---
Write-Host ""
Write-Host "[2/3] starting hexrun bench and sampling power..." -ForegroundColor DarkGray
$benchArgs = @("bench", $Model, "--repeats", "2")
$proc = Start-Process -FilePath $Hexrun -ArgumentList $benchArgs -PassThru -RedirectStandardOutput ".\.energy-bench-stdout.log" -RedirectStandardError ".\.energy-bench-stderr.log" -WindowStyle Hidden

# Sample until the process exits.
$busy = [System.Collections.Generic.List[double]]::new()
$start = Get-Date
while (-not $proc.HasExited) {
    $mw = Get-DischargeRateMilliwatts
    if ($null -ne $mw) { [void]$busy.Add([double]$mw) }
    Start-Sleep -Milliseconds $SampleIntervalMs
}
$elapsed = ((Get-Date) - $start).TotalSeconds
$busy_mean_mw = Mean $busy
Write-Host ("       busy mean: {0:N0} mW ({1:N2} W) over {2} samples ({3:N1} s)" -f $busy_mean_mw, ($busy_mean_mw / 1000), $busy.Count, $elapsed)

# Give Windows a beat to flush the redirected stdout buffer before we read it.
Start-Sleep -Milliseconds 800

# --- compute delta + parse hexrun bench stdout for token totals ---
$delta_w = ($busy_mean_mw - $idle_mean_mw) / 1000.0
$inference_energy_j = $delta_w * $elapsed

# Parse tokens directly from the per-query lines. Don't depend on the
# summary line existing -- buffered I/O sometimes truncates the tail.
$bench_stdout = ""
if (Test-Path ".\.energy-bench-stdout.log") {
    $bench_stdout = Get-Content ".\.energy-bench-stdout.log" -Raw -ErrorAction SilentlyContinue
}
if (-not $bench_stdout) { $bench_stdout = "" }
$tok_matches = [regex]::Matches($bench_stdout, "response \((\d+) approx tokens\)")
$tokens_total = if ($tok_matches.Count -gt 0) {
    ($tok_matches | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
} else { $null }
$tps_match = [regex]::Match($bench_stdout, "aggregate tok/s \(post ttft\):\s+([0-9.]+)")
$tps_post = if ($tps_match.Success) { [double]$tps_match.Groups[1].Value } else { $null }

Write-Host ""
Write-Host "[3/3] results" -ForegroundColor Cyan
Write-Host "----------------------------------------"
Write-Host ("  idle baseline:                  {0,8:N2} W" -f ($idle_mean_mw / 1000))
Write-Host ("  during inference:               {0,8:N2} W" -f ($busy_mean_mw / 1000))
Write-Host ("  inference delta:                {0,8:N2} W" -f $delta_w)
Write-Host ("  total time:                     {0,8:N2} s" -f $elapsed)
Write-Host ("  total inference energy (delta): {0,8:N2} J" -f $inference_energy_j)
if ($null -ne $tokens_total -and $tokens_total -gt 0) {
    $j_per_token = $inference_energy_j / $tokens_total
    Write-Host ("  approx tokens generated:        {0,8}" -f $tokens_total)
    Write-Host ("  joules per token (delta):       {0,8:N2} J/token" -f $j_per_token)
    if ($null -ne $tps_post) {
        Write-Host ("  hexrun aggregate tok/s:         {0,8:N1} tok/s (post ttft)" -f $tps_post)
    }
} else {
    Write-Host "  (could not parse token count from bench stdout; inspect .energy-bench-stdout.log)"
}
Write-Host ""
Write-Host "raw stdout in .\.energy-bench-stdout.log; stderr in .\.energy-bench-stderr.log" -ForegroundColor DarkGray
Write-Host ""

if ($delta_w -lt 0.5) {
    Write-Host "Note: delta is small ($([math]::Round($delta_w,2)) W). Battery telemetry is noisy at low loads; consider running on a deeper baseline or with more samples." -ForegroundColor Yellow
}

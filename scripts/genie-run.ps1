# scripts/genie-run.ps1
#
# Loads the QAIRT environment and runs `genie-t2t-run.exe` against a Genie
# bundle directory. Designed for Phase 0 NPU validation: prove the Hexagon
# NPU executes a real LLM end-to-end.
#
# Usage:
#   pwsh -File scripts\genie-run.ps1 -Bundle "C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite" -Prompt "Tell me a joke"
#
# Defaults: the most recently exported Qwen 2.5 7B bundle, a one-line prompt.

[CmdletBinding()]
param(
    [string]$Bundle = "C:\AAA\Personal\AI\models\qwen-2.5-7b\bundle\qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite",
    [string]$QairtRoot = $env:QNN_SDK_ROOT,
    [string]$Prompt = "Tell me a short joke about Snapdragon laptops.",
    [string]$ConfigName = "genie_config.json"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($QairtRoot)) {
    $QairtRoot = "C:\AAA\Personal\AI\qairt\2.45.0"
    Write-Host "QairtRoot not provided; defaulting to $QairtRoot" -ForegroundColor DarkGray
}
if (-not (Test-Path $QairtRoot)) {
    throw "QAIRT SDK directory not found at $QairtRoot. Set -QairtRoot or QNN_SDK_ROOT."
}
if (-not (Test-Path $Bundle)) {
    throw "Bundle directory not found at $Bundle"
}
$ConfigPath = Join-Path $Bundle $ConfigName
if (-not (Test-Path $ConfigPath)) {
    throw "$ConfigName not found in bundle dir $Bundle"
}

# Wire up the runtime environment per the Qualcomm LLM-on-Genie tutorial.
$env:QAIRT_HOME = $QairtRoot
$env:Path = "$QairtRoot\bin\aarch64-windows-msvc;$QairtRoot\lib\aarch64-windows-msvc;" + $env:Path
$env:ADSP_LIBRARY_PATH = "$QairtRoot\lib\hexagon-v73\unsigned"

Write-Host ""
Write-Host "==  hexrun Phase 0 smoke test  ==" -ForegroundColor Cyan
Write-Host "QAIRT_HOME      = $env:QAIRT_HOME"
Write-Host "Bundle          = $Bundle"
Write-Host "Config          = $ConfigPath"
Write-Host "ADSP_LIBRARY    = $env:ADSP_LIBRARY_PATH"
Write-Host ""
Write-Host "Prompt: $Prompt" -ForegroundColor Yellow
Write-Host ""
Write-Host "Watching the NPU? Open Task Manager > Performance > NPU before tokens stream." -ForegroundColor DarkGray
Write-Host ""

# Use Qwen's chat template wrappers around the user prompt so the model produces a chat reply.
$Wrapped = "<|im_start|>system`nYou are a helpful assistant.<|im_end|>`n<|im_start|>user`n$Prompt<|im_end|>`n<|im_start|>assistant`n"

Push-Location $Bundle
try {
    & genie-t2t-run.exe -c $ConfigName -p $Wrapped
} finally {
    Pop-Location
}

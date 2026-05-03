# Bumps the Windows system page file. Useful when running local
# qai-hub-models exports — ONNX consolidation can blow past 16 GB RAM.
# Defaults give ~40 GB swap; tune for the disk you have.
# Must run elevated. Reboot after to take full effect.
param(
    [int]$InitialMB = 32768,
    [int]$MaximumMB = 40960
)

$ErrorActionPreference = 'Stop'

$logDir = Join-Path -Path $PSScriptRoot -ChildPath '..\logs'
$null = New-Item -ItemType Directory -Force -Path $logDir
Start-Transcript -Path (Join-Path $logDir 'set-pagefile.log') -Force

Write-Host "Setting page file: Initial=$InitialMB MB  Maximum=$MaximumMB MB"

$cs = Get-CimInstance Win32_ComputerSystem
if ($cs.AutomaticManagedPagefile) {
    Write-Host "Disabling AutomaticManagedPagefile..."
    Set-CimInstance -InputObject $cs -Property @{AutomaticManagedPagefile = $false}
}

$pf = Get-CimInstance Win32_PageFileSetting -Filter "Name='c:\\pagefile.sys'" -ErrorAction SilentlyContinue
if ($pf) {
    Write-Host "Removing existing 0/0 setting first."
    Remove-CimInstance -InputObject $pf
}
Write-Host "Creating fresh setting."
$pf = New-CimInstance -ClassName Win32_PageFileSetting -Property @{
    Name        = 'c:\pagefile.sys'
    InitialSize = [uint32]$InitialMB
    MaximumSize = [uint32]$MaximumMB
}

Write-Host "---After---"
Get-CimInstance Win32_PageFileSetting | Format-List Name,InitialSize,MaximumSize
Write-Host "Reboot required for the change to take full effect."
Stop-Transcript

# scripts/chat.ps1
#
# Tiny multi-turn chat REPL against a running `npurun serve`. Streams
# replies as they generate. Useful when you want to talk to npurun
# from a CLI without spinning up a browser-based chat UI.
#
# Usage:
#   pwsh -File scripts\chat.ps1
#   pwsh -File scripts\chat.ps1 -Addr 127.0.0.1:11435 -Model phi-3.5-mini
#
# Commands inside the REPL:
#   /exit   — quit
#   /reset  — clear conversation history
#   /save   — save the transcript to chat-<timestamp>.txt
#   /sys    — set / replace the system prompt
#
# Requires `curl` on PATH (ships with Windows 11 build 17063+).

[CmdletBinding()]
param(
    [string]$Addr = $(if ($env:NPURUN_SERVE_ADDR) { $env:NPURUN_SERVE_ADDR } else { "127.0.0.1:11435" }),
    [string]$Model = "phi-3.5-mini",
    [string]$AuthToken = "",
    [int]$MaxTokens = 512,
    [string]$System = ""
)

$ErrorActionPreference = "Stop"
$baseUrl = "http://$Addr"

function Write-Banner($text, $color = "DarkGray") {
    Write-Host $text -ForegroundColor $color
}

# --- probe server ---
try {
    $headers = @{}
    if ($AuthToken) { $headers["Authorization"] = "Bearer $AuthToken" }
    $health = Invoke-RestMethod -Uri "$baseUrl/healthz" -Headers $headers -TimeoutSec 5
} catch {
    Write-Host ""
    Write-Host "Cannot reach $baseUrl/healthz." -ForegroundColor Red
    Write-Host "Start the server first:" -ForegroundColor Red
    Write-Host "    npurun serve --model $Model"
    Write-Host ""
    exit 1
}

Write-Host ""
Write-Banner "==  npurun chat REPL  =="
Write-Banner "  server:   $($health.status) ($baseUrl)"
Write-Banner "  model:    $($health.model)"
Write-Banner "  uptime:   $($health.uptime_seconds)s"
Write-Banner "  auth:     $(if ($health.auth) { 'on' } else { 'off' })"
Write-Banner ""
Write-Banner "  /exit, /reset, /save, /sys <prompt>"
Write-Host ""

# --- conversation state ---
$messages = New-Object System.Collections.Generic.List[hashtable]
if ($System) {
    $messages.Add(@{role = "system"; content = $System}) | Out-Null
}

# --- main loop ---
while ($true) {
    Write-Host "you> " -NoNewline -ForegroundColor Cyan
    $line = [Console]::In.ReadLine()
    if ($null -eq $line) { break }
    $trimmed = $line.Trim()
    if ([string]::IsNullOrEmpty($trimmed)) { continue }

    if ($trimmed -eq "/exit") { break }

    if ($trimmed -eq "/reset") {
        $messages.Clear()
        if ($System) { $messages.Add(@{role = "system"; content = $System}) | Out-Null }
        Write-Banner "(history cleared)"
        continue
    }

    if ($trimmed -eq "/save") {
        $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $path = "chat-$stamp.txt"
        $messages | ForEach-Object { "$($_.role)> $($_.content)" } | Set-Content -Path $path -Encoding utf8
        Write-Banner "(transcript saved to $path)"
        continue
    }

    if ($trimmed.StartsWith("/sys ")) {
        $newSys = $trimmed.Substring(5).Trim()
        # Replace existing system or prepend a new one.
        if ($messages.Count -gt 0 -and $messages[0].role -eq "system") {
            $messages[0] = @{role = "system"; content = $newSys}
        } else {
            $messages.Insert(0, @{role = "system"; content = $newSys})
        }
        $System = $newSys
        Write-Banner "(system prompt set)"
        continue
    }

    # Normal user turn — send to the server, stream the reply.
    $messages.Add(@{role = "user"; content = $trimmed}) | Out-Null

    $payload = @{
        model       = $Model
        messages    = @($messages)
        stream      = $true
        max_tokens  = $MaxTokens
    } | ConvertTo-Json -Depth 4 -Compress

    $tmpPayload = New-TemporaryFile
    Set-Content -Path $tmpPayload -Value $payload -Encoding utf8 -NoNewline

    $curlArgs = @(
        "--silent",
        "--no-buffer",
        "-X", "POST",
        "$baseUrl/v1/chat/completions",
        "-H", "Content-Type: application/json"
    )
    if ($AuthToken) {
        $curlArgs += @("-H", "Authorization: Bearer $AuthToken")
    }
    $curlArgs += @("--data-binary", "@$tmpPayload")

    Write-Host "npurun> " -NoNewline -ForegroundColor Green
    $reply = New-Object System.Text.StringBuilder

    & curl.exe @curlArgs | ForEach-Object {
        $sseLine = $_
        if ([string]::IsNullOrEmpty($sseLine)) { return }
        if (-not $sseLine.StartsWith("data:")) { return }
        $data = $sseLine.Substring(5).Trim()
        if ($data -eq "[DONE]") { return }
        try {
            $chunk = $data | ConvertFrom-Json
        } catch { return }
        if (-not $chunk.choices) { return }
        $delta = $chunk.choices[0].delta
        if ($delta -and $delta.content) {
            Write-Host $delta.content -NoNewline
            [void]$reply.Append($delta.content)
        }
    }

    Remove-Item -Path $tmpPayload -Force -ErrorAction SilentlyContinue

    Write-Host ""
    Write-Host ""

    $assistantText = $reply.ToString().Trim()
    if (-not [string]::IsNullOrEmpty($assistantText)) {
        $messages.Add(@{role = "assistant"; content = $assistantText}) | Out-Null
    } else {
        # No content came back — drop the user message so the next turn
        # doesn't carry a dangling user with no assistant reply.
        $messages.RemoveAt($messages.Count - 1)
        Write-Banner "(no reply — server may have errored, check the serve window)"
    }
}

Write-Host ""
Write-Banner "bye"

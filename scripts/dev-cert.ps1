# scripts/dev-cert.ps1
#
# Generate a self-signed code-signing cert for *development* MSIX builds
# and (optionally) trust it on the local machine. With this in place,
# `scripts\build-msix.ps1 -CertThumbprint <thumb>` produces an MSIX that
# installs by double-click on the same laptop without needing developer
# mode.
#
# Self-signed means: only this machine (and machines that have explicitly
# imported the cert into their Trusted People / Trusted Root stores) will
# accept the MSIX. Public releases need a real EV / standard cert from a
# CA — that's Phase 6 final, not this script.
#
# Usage:
#   pwsh -File scripts\dev-cert.ps1                # generate, trust, print thumbprint
#   pwsh -File scripts\dev-cert.ps1 -ExportPath .\dist\npurun-dev-cert.cer
#   pwsh -File scripts\dev-cert.ps1 -List          # show existing dev certs
#   pwsh -File scripts\dev-cert.ps1 -Remove <thumb>

[CmdletBinding(DefaultParameterSetName = "Generate")]
param(
    # Subject CN — must match AppxManifest.xml's Publisher attribute.
    [Parameter(ParameterSetName = "Generate")]
    [string]$Subject = "CN=Brenden Bonker",

    # Validity in years.
    [Parameter(ParameterSetName = "Generate")]
    [int]$Years = 1,

    # Optional path to export the public cert (.cer) so other machines
    # can trust it.
    [Parameter(ParameterSetName = "Generate")]
    [string]$ExportPath,

    # Skip the trust step (useful for CI setups that import the cert
    # via a different path).
    [Parameter(ParameterSetName = "Generate")]
    [switch]$NoTrust,

    [Parameter(ParameterSetName = "List")]
    [switch]$List,

    [Parameter(ParameterSetName = "Remove", Mandatory = $true)]
    [string]$Remove
)

$ErrorActionPreference = "Stop"
$store = "Cert:\CurrentUser\My"

function Find-DevCerts {
    Get-ChildItem $store | Where-Object {
        $_.Subject -like "CN=Brenden*" -and $_.HasPrivateKey -and
        ($_.EnhancedKeyUsageList.ObjectId -contains "1.3.6.1.5.5.7.3.3")
    }
}

if ($PSCmdlet.ParameterSetName -eq "List") {
    $certs = Find-DevCerts
    if (-not $certs) {
        Write-Host "no dev code-signing certs in $store"
        exit 0
    }
    foreach ($c in $certs) {
        Write-Host ""
        Write-Host "  thumbprint:  $($c.Thumbprint)"
        Write-Host "  subject:     $($c.Subject)"
        Write-Host "  not-after:   $($c.NotAfter.ToString('yyyy-MM-dd'))"
    }
    exit 0
}

if ($PSCmdlet.ParameterSetName -eq "Remove") {
    $path = Join-Path $store $Remove
    if (-not (Test-Path $path)) {
        Write-Error "no cert with thumbprint $Remove in $store"
    }
    Remove-Item $path -Force
    Write-Host "removed $Remove"
    exit 0
}

# --- generate ---
Write-Host "==  npurun dev cert  ==" -ForegroundColor Cyan
Write-Host "  subject:   $Subject"
Write-Host "  validity:  $Years year(s)"

$existing = Find-DevCerts | Where-Object Subject -EQ $Subject
if ($existing) {
    Write-Host ""
    Write-Host "found existing cert(s) with this subject:" -ForegroundColor Yellow
    foreach ($c in $existing) {
        Write-Host "  $($c.Thumbprint)  not-after $($c.NotAfter.ToString('yyyy-MM-dd'))"
    }
    Write-Host ""
    Write-Host "  pass -Remove <thumb> to delete one before generating a new one"
}

$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject $Subject `
    -KeyUsage DigitalSignature `
    -FriendlyName "npurun dev signing cert" `
    -CertStoreLocation $store `
    -NotAfter (Get-Date).AddYears($Years) `
    -KeyAlgorithm RSA `
    -KeyLength 2048 `
    -HashAlgorithm SHA256

Write-Host ""
Write-Host "generated cert:" -ForegroundColor Green
Write-Host "  thumbprint:  $($cert.Thumbprint)"
Write-Host "  subject:     $($cert.Subject)"
Write-Host "  not-after:   $($cert.NotAfter.ToString('yyyy-MM-dd'))"

if (-not $NoTrust) {
    # For a self-signed code-signing cert to be trusted by Windows for
    # MSIX install, it needs to live in BOTH:
    # - CurrentUser\My (already there from New-SelfSignedCertificate)
    # - LocalMachine\TrustedPeople (where MSIX install pulls signer trust)
    # The latter requires admin. Try; fall back with a clear message.
    try {
        $tmpCer = Join-Path $env:TEMP "npurun-dev-cert-$($cert.Thumbprint).cer"
        Export-Certificate -Cert $cert -FilePath $tmpCer | Out-Null
        Import-Certificate -FilePath $tmpCer -CertStoreLocation Cert:\LocalMachine\TrustedPeople -ErrorAction Stop | Out-Null
        Remove-Item $tmpCer -Force
        Write-Host "  trusted in LocalMachine\TrustedPeople (MSIX install will accept it)"
    } catch {
        Write-Host ""
        Write-Host "  !!  could not import into LocalMachine\TrustedPeople (admin needed)" -ForegroundColor Yellow
        Write-Host "      open an elevated PowerShell and run:" -ForegroundColor Yellow
        Write-Host "        Import-Certificate -FilePath '<exported.cer>' -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
    }
}

if ($ExportPath) {
    Export-Certificate -Cert $cert -FilePath $ExportPath | Out-Null
    Write-Host "  exported public cert to $ExportPath"
}

Write-Host ""
Write-Host "to build a signed MSIX with this cert:" -ForegroundColor Cyan
Write-Host "  pwsh -File scripts\build-msix.ps1 -CertThumbprint $($cert.Thumbprint)"
Write-Host ""

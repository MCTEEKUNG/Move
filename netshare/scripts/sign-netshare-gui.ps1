param(
    [string]$ExePath,
    [string]$PfxPath,
    [string]$PfxPassword,
    [string]$CertThumbprint,
    [switch]$SkipTimestamp,
    [string]$TimestampServer = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if (-not $ExePath) {
    $ExePath = Join-Path $repoRoot "target\release\netshare-gui.exe"
}
$resolvedExePath = [System.IO.Path]::GetFullPath($ExePath)

if (-not (Test-Path $resolvedExePath)) {
    throw "Executable not found: $resolvedExePath"
}

$certificate = $null

if (-not [string]::IsNullOrWhiteSpace($CertThumbprint)) {
    $certificate = Get-ChildItem "Cert:\CurrentUser\My" |
        Where-Object { $_.Thumbprint -eq $CertThumbprint -and $_.HasPrivateKey } |
        Select-Object -First 1

    if (-not $certificate) {
        throw "Certificate with private key not found in Cert:\CurrentUser\My for thumbprint $CertThumbprint"
    }
}
else {
    if ([string]::IsNullOrWhiteSpace($PfxPath)) {
        throw "Provide either -CertThumbprint or (-PfxPath and -PfxPassword)"
    }

    $resolvedPfxPath = [System.IO.Path]::GetFullPath($PfxPath)
    if (-not (Test-Path $resolvedPfxPath)) {
        throw "PFX file not found: $resolvedPfxPath"
    }
    if ([string]::IsNullOrWhiteSpace($PfxPassword)) {
        throw "PfxPassword is required when using PfxPath"
    }

    $securePassword = ConvertTo-SecureString -String $PfxPassword -AsPlainText -Force
    $imported = Import-PfxCertificate -FilePath $resolvedPfxPath -CertStoreLocation "Cert:\CurrentUser\My" -Password $securePassword -Exportable
    $certificate = $imported | Select-Object -First 1

    if (-not $certificate -or -not $certificate.HasPrivateKey) {
        throw "Failed to import signing certificate with private key"
    }
}

if ($SkipTimestamp) {
    $result = Set-AuthenticodeSignature -FilePath $resolvedExePath -Certificate $certificate
}
else {
    $result = Set-AuthenticodeSignature -FilePath $resolvedExePath -Certificate $certificate -TimestampServer $TimestampServer
}

Write-Host ("Signature status: {0}" -f $result.Status)
Write-Host ("Signer: {0}" -f $result.SignerCertificate.Subject)
Write-Host ("Thumbprint: {0}" -f $result.SignerCertificate.Thumbprint)

if ($result.Status -ne "Valid") {
    throw "Signing failed with status: $($result.Status)"
}

$verify = Get-AuthenticodeSignature -FilePath $resolvedExePath
Write-Host ("Post-check status: {0}" -f $verify.Status)
if ($verify.Status -ne "Valid") {
    throw "Post-sign verification failed with status: $($verify.Status)"
}

Write-Host ("Signed executable: {0}" -f $resolvedExePath)

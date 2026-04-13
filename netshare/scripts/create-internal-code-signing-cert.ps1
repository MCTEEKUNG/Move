param(
    [string]$Subject = "CN=NetShare Internal Code Signing",
    [int]$ValidYears = 5,
    [string]$OutputDirectory,
    [string]$PfxPassword
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot "certs\internal-code-signing"
}
$resolvedOutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $resolvedOutputDirectory | Out-Null

if ($ValidYears -lt 1) {
    throw "ValidYears must be >= 1"
}

$cert = New-SelfSignedCertificate `
    -Type CodeSigningCert `
    -Subject $Subject `
    -CertStoreLocation "Cert:\CurrentUser\My" `
    -HashAlgorithm "SHA256" `
    -KeyAlgorithm "RSA" `
    -KeyLength 4096 `
    -KeyExportPolicy Exportable `
    -NotAfter (Get-Date).AddYears($ValidYears) `
    -FriendlyName "NetShare Internal Code Signing"

$baseName = "netshare-internal-code-signing"
$cerPath = Join-Path $resolvedOutputDirectory ("{0}.cer" -f $baseName)
$pfxPath = Join-Path $resolvedOutputDirectory ("{0}.pfx" -f $baseName)

Export-Certificate -Cert $cert -FilePath $cerPath | Out-Null

if ([string]::IsNullOrWhiteSpace($PfxPassword)) {
    $securePassword = Read-Host "Enter password for PFX export" -AsSecureString
}
else {
    $securePassword = ConvertTo-SecureString -String $PfxPassword -AsPlainText -Force
}

Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $securePassword | Out-Null

Write-Host "Created internal code-signing certificate"
Write-Host ("Subject: {0}" -f $cert.Subject)
Write-Host ("Thumbprint: {0}" -f $cert.Thumbprint)
Write-Host ("CER: {0}" -f $cerPath)
Write-Host ("PFX: {0}" -f $pfxPath)
Write-Host "Copy the CER file to target machines and install it with install-internal-code-signing-cert.ps1"

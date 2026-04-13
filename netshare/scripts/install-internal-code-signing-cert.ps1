param(
    [Parameter(Mandatory = $true)]
    [string]$CerPath,
    [switch]$InstallForAllUsers
)

$ErrorActionPreference = "Stop"

$resolvedCerPath = [System.IO.Path]::GetFullPath($CerPath)
if (-not (Test-Path $resolvedCerPath)) {
    throw "Certificate file not found: $resolvedCerPath"
}

if ($InstallForAllUsers) {
    $rootStorePath = "Cert:\LocalMachine\Root"
    $publisherStorePath = "Cert:\LocalMachine\TrustedPublisher"
}
else {
    $rootStorePath = "Cert:\CurrentUser\Root"
    $publisherStorePath = "Cert:\CurrentUser\TrustedPublisher"
}

$importedRoot = Import-Certificate -FilePath $resolvedCerPath -CertStoreLocation $rootStorePath
$importedPublisher = Import-Certificate -FilePath $resolvedCerPath -CertStoreLocation $publisherStorePath

Write-Host "Installed internal signing certificate"
Write-Host ("Thumbprint: {0}" -f $importedRoot.Certificate.Thumbprint)
Write-Host ("Root store: {0}" -f $rootStorePath)
Write-Host ("Trusted Publisher store: {0}" -f $publisherStorePath)

$thumbprint = $importedRoot.Certificate.Thumbprint
$rootCheck = Get-ChildItem $rootStorePath | Where-Object { $_.Thumbprint -eq $thumbprint }
$publisherCheck = Get-ChildItem $publisherStorePath | Where-Object { $_.Thumbprint -eq $thumbprint }

if (-not $rootCheck -or -not $publisherCheck) {
    throw "Certificate verification failed after import"
}

Write-Host "Verification OK"

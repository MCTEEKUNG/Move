param(
	[ValidateSet("release", "debug", "perf-test")]
	[string]$Profile = "release",
	[string]$DestinationPath,
	[string]$PfxPath,
	[string]$PfxPassword,
	[string]$CertThumbprint,
	[switch]$SkipTimestamp,
	[string]$TimestampServer = "http://timestamp.digicert.com",
	[switch]$IncludePdb,
	[switch]$Zip
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$guiExeName = "netshare-gui.exe"
$logScriptName = "run-netshare-gui-with-logging.ps1"
$signScriptName = "sign-netshare-gui.ps1"

if (-not $DestinationPath) {
	$DestinationPath = Join-Path $repoRoot ("stage\gui-{0}" -f $Profile)
}
$resolvedDestination = [System.IO.Path]::GetFullPath($DestinationPath)

$hasThumbprint = -not [string]::IsNullOrWhiteSpace($CertThumbprint)
$hasPfx = -not [string]::IsNullOrWhiteSpace($PfxPath)

if (-not $hasThumbprint -and -not $hasPfx) {
	throw "Provide signing input with either -CertThumbprint or (-PfxPath and -PfxPassword)"
}
if ($hasPfx -and [string]::IsNullOrWhiteSpace($PfxPassword)) {
	throw "PfxPassword is required when using PfxPath"
}

Push-Location $repoRoot
try {
	Write-Host ("[1/3] Building netshare-gui ({0})..." -f $Profile)
	if ($Profile -eq "release") {
		cargo build --release -p netshare-gui
	}
	else {
		cargo build --profile $Profile -p netshare-gui
	}
	if ($LASTEXITCODE -ne 0) {
		throw "cargo build failed with exit code $LASTEXITCODE"
	}

	$profileOutputDir = if ($Profile -eq "release") {
		Join-Path $repoRoot "target\release"
	}
	else {
		Join-Path $repoRoot ("target\{0}" -f $Profile)
	}
	$guiExePath = Join-Path $profileOutputDir $guiExeName
	$pdbPath = Join-Path $profileOutputDir "netshare_gui.pdb"

	if (-not (Test-Path $guiExePath)) {
		throw "GUI binary not found at '$guiExePath'"
	}

	Write-Host "[2/3] Signing executable..."
	$signScriptPath = Join-Path $PSScriptRoot $signScriptName
	if (-not (Test-Path $signScriptPath)) {
		throw "Signing script not found at '$signScriptPath'"
	}

	$signParams = @{
		ExePath = $guiExePath
		TimestampServer = $TimestampServer
	}
	if ($SkipTimestamp) {
		$signParams["SkipTimestamp"] = $true
	}
	if ($hasThumbprint) {
		$signParams["CertThumbprint"] = $CertThumbprint
	}
	else {
		$signParams["PfxPath"] = $PfxPath
		$signParams["PfxPassword"] = $PfxPassword
	}
	& $signScriptPath @signParams

	Write-Host "[3/3] Staging test bundle..."
	$logScriptPath = Join-Path $PSScriptRoot $logScriptName
	if (-not (Test-Path $logScriptPath)) {
		throw "Logging script not found at '$logScriptPath'"
	}

	New-Item -ItemType Directory -Force -Path $resolvedDestination | Out-Null

	Copy-Item $guiExePath (Join-Path $resolvedDestination $guiExeName) -Force
	Copy-Item $logScriptPath (Join-Path $resolvedDestination $logScriptName) -Force

	if ($IncludePdb -and (Test-Path $pdbPath)) {
		Copy-Item $pdbPath (Join-Path $resolvedDestination "netshare_gui.pdb") -Force
	}

	$launcherPath = Join-Path $resolvedDestination "RUN-NETSHARE-GUI-LOGGING.bat"
	@(
		"@echo off"
		"cd /d %~dp0"
		"powershell -ExecutionPolicy Bypass -File .\run-netshare-gui-with-logging.ps1 -GuiPath .\netshare-gui.exe"
	) | Set-Content -Path $launcherPath -Encoding ASCII

	$readmePath = Join-Path $resolvedDestination "RUN-TEST.txt"
	@(
		"NetShare GUI signed test bundle"
		""
		("Profile: {0}" -f $Profile)
		("Built at: {0}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"))
		""
		"Target machine quick start:"
		"1. Copy this whole folder to the target machine"
		"2. Double-click RUN-NETSHARE-GUI-LOGGING.bat"
		"3. After test, collect newest file from .\\logs"
	) | Set-Content -Path $readmePath -Encoding ASCII

	if ($Zip) {
		$zipPath = "$resolvedDestination.zip"
		if (Test-Path $zipPath) {
			Remove-Item $zipPath -Force
		}
		Compress-Archive -Path (Join-Path $resolvedDestination "*") -DestinationPath $zipPath
		Write-Host ("Created archive: {0}" -f $zipPath)
	}

	Write-Host ("Staged GUI bundle at: {0}" -f $resolvedDestination)
}
finally {
	Pop-Location
}

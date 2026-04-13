param(
    [string]$GuiPath,
    [string]$LogDirectory,
    [string]$RustLog = "netshare::latency=debug,netshare=info",
    [switch]$NoWait
)

$ErrorActionPreference = "Stop"

if (-not $GuiPath) {
    $GuiPath = Join-Path $PSScriptRoot "..\target\release\netshare-gui.exe"
}

$resolvedGuiPath = [System.IO.Path]::GetFullPath($GuiPath)

if (-not (Test-Path $resolvedGuiPath)) {
    throw "netshare-gui.exe not found at '$resolvedGuiPath'"
}

if (-not $LogDirectory) {
    $LogDirectory = Join-Path ([System.IO.Path]::GetDirectoryName($resolvedGuiPath)) "logs"
}

$resolvedLogDirectory = [System.IO.Path]::GetFullPath($LogDirectory)
New-Item -ItemType Directory -Force -Path $resolvedLogDirectory | Out-Null

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $resolvedLogDirectory ("netshare-gui-{0}.log" -f $timestamp)

Write-Host "Launching $resolvedGuiPath"
Write-Host "Writing logs to $logPath"
Write-Host "RUST_LOG=$RustLog"

$previousRustLog = $env:RUST_LOG
$previousBacktrace = $env:RUST_BACKTRACE

try {
    $env:RUST_LOG = $RustLog
    $env:RUST_BACKTRACE = "1"

    if ($NoWait) {
        $process = Start-Process -FilePath $resolvedGuiPath -WorkingDirectory ([System.IO.Path]::GetDirectoryName($resolvedGuiPath)) -RedirectStandardOutput $logPath -RedirectStandardError $logPath -PassThru
        Write-Host ("NetShare GUI started with PID {0}" -f $process.Id)
        Write-Host "Use Stop-Process or close the app window when finished."
    }
    else {
        & $resolvedGuiPath *>> $logPath
        $exitCode = $LASTEXITCODE
        if ($null -ne $exitCode) {
            Write-Host ("netshare-gui exited with code {0}" -f $exitCode)
        }
    }
}
finally {
    if ($null -eq $previousRustLog) {
        Remove-Item Env:RUST_LOG -ErrorAction SilentlyContinue
    }
    else {
        $env:RUST_LOG = $previousRustLog
    }

    if ($null -eq $previousBacktrace) {
        Remove-Item Env:RUST_BACKTRACE -ErrorAction SilentlyContinue
    }
    else {
        $env:RUST_BACKTRACE = $previousBacktrace
    }
}
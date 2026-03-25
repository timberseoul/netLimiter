# Run script for NetLimiter
# Must be run as Administrator (WinDivert requires it)

$ErrorActionPreference = "Stop"

$buildDir = "$PSScriptRoot\..\build"

if (-not (Test-Path "$buildDir\netlimiter-core.exe")) {
    Write-Host "Build not found. Run build.ps1 first." -ForegroundColor Red
    exit 1
}

Write-Host "=== Starting NetLimiter ===" -ForegroundColor Cyan
Write-Host "Starting Rust capture engine (requires Administrator)..." -ForegroundColor Yellow

# Start the Rust core in background
$rustProcess = Start-Process -FilePath "$buildDir\netlimiter-core.exe" `
    -WorkingDirectory $buildDir `
    -PassThru `
    -WindowStyle Hidden

Write-Host "Rust core started (PID: $($rustProcess.Id))" -ForegroundColor Green

# Wait a moment for the pipe server to be ready
Start-Sleep -Seconds 2

Write-Host "Starting TUI..." -ForegroundColor Yellow

# Run the Go TUI in foreground
& "$buildDir\netlimiter-ui.exe"

# When TUI exits, stop the Rust core
Write-Host "`nStopping Rust core..." -ForegroundColor Yellow
if (-not $rustProcess.HasExited) {
    Stop-Process -Id $rustProcess.Id -Force
}
Write-Host "Done." -ForegroundColor Green

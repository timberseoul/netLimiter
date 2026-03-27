# Run script for NetLimiter
# Must be run as Administrator (WinDivert requires it)
#
# The UI now automatically launches and manages the Rust core process.
# You only need to run one executable.

$ErrorActionPreference = "Stop"

$buildDir = "$PSScriptRoot\..\build"

if (-not (Test-Path "$buildDir\netlimiter-ui.exe")) {
    Write-Host "Build not found. Run build.ps1 first." -ForegroundColor Red
    exit 1
}

Write-Host "=== Starting NetLimiter ===" -ForegroundColor Cyan
Write-Host "UI will automatically launch the core engine." -ForegroundColor Yellow

# Run the UI — it handles core lifecycle automatically
& "$buildDir\netlimiter-ui.exe"

Write-Host "Done." -ForegroundColor Green

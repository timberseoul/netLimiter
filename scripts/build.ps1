# Build script for NetLimiter
# Run as Administrator

$ErrorActionPreference = "Stop"

Write-Host "=== Building NetLimiter ===" -ForegroundColor Cyan

# Kill any running instances first to release file locks
$procs = @("netlimiter-core", "netlimiter-ui")
foreach ($p in $procs) {
    $running = Get-Process -Name $p -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "Stopping running $p..." -ForegroundColor Yellow
        Stop-Process -Name $p -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
    }
}

# Build Rust core
Write-Host "`n[1/2] Building Rust core..." -ForegroundColor Yellow
Push-Location "$PSScriptRoot\..\rust_core"
$env:WINDIVERT_PATH = (Resolve-Path ".\libs").Path
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "Rust build failed!" -ForegroundColor Red
    Pop-Location
    exit 1
}
Pop-Location
Write-Host "Rust core built successfully." -ForegroundColor Green

# Build Go TUI
Write-Host "`n[2/2] Building Go TUI..." -ForegroundColor Yellow
Push-Location "$PSScriptRoot\..\go-ui"
go mod tidy
go build -ldflags="-s -w" -o "..\build\netlimiter-ui.exe" .
if ($LASTEXITCODE -ne 0) {
    Write-Host "Go build failed!" -ForegroundColor Red
    Pop-Location
    exit 1
}
Pop-Location
Write-Host "Go TUI built successfully." -ForegroundColor Green

# Copy artifacts to build directory
$buildDir = "$PSScriptRoot\..\build"
New-Item -ItemType Directory -Force -Path $buildDir | Out-Null

$filesToCopy = @(
    @{ Src = "$PSScriptRoot\..\rust_core\target\release\netlimiter-core.exe"; Dst = "$buildDir\" },
    @{ Src = "$PSScriptRoot\..\rust_core\libs\WinDivert.dll";                Dst = "$buildDir\" },
    @{ Src = "$PSScriptRoot\..\rust_core\libs\WinDivert64.sys";              Dst = "$buildDir\" }
)

foreach ($f in $filesToCopy) {
    for ($i = 0; $i -lt 3; $i++) {
        try {
            Copy-Item $f.Src $f.Dst -Force
            break
        } catch {
            if ($i -eq 2) {
                Write-Host "Warning: Could not copy $($f.Src): $_" -ForegroundColor Yellow
            } else {
                Start-Sleep -Milliseconds 500
            }
        }
    }
}

Write-Host "`n=== Build complete ===" -ForegroundColor Cyan
Write-Host "Output: $buildDir" -ForegroundColor White
Write-Host "  - netlimiter-core.exe  (Rust capture engine)" -ForegroundColor White
Write-Host "  - netlimiter-ui.exe    (Go TUI)" -ForegroundColor White
Write-Host "  - WinDivert.dll        (WinDivert library)" -ForegroundColor White
Write-Host "  - WinDivert64.sys      (WinDivert driver)" -ForegroundColor White

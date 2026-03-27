# Build script for NetLimiter
# Run as Administrator

$ErrorActionPreference = "Stop"

Write-Host "=== Building NetLimiter ===" -ForegroundColor Cyan

$uiSourcePng = "$PSScriptRoot\..\assets\FxemojiLightningmood.png"
$coreSourcePng = "$PSScriptRoot\..\assets\LogosCoreosIcon.png"
$uiIconIco = "$PSScriptRoot\..\assets\app-icon-ui.ico"
$coreIconIco = "$PSScriptRoot\..\assets\app-icon-core.ico"
$goSyso = "$PSScriptRoot\..\go-ui\rsrc_windows_amd64.syso"

function Write-PngIco {
    param(
        [Parameter(Mandatory = $true)] [byte[]] $PngBytes,
        [Parameter(Mandatory = $true)] [string] $OutputPath,
        [Parameter(Mandatory = $true)] [int] $Width,
        [Parameter(Mandatory = $true)] [int] $Height
    )

    $fs = [System.IO.File]::Open($OutputPath, [System.IO.FileMode]::Create)
    try {
        $bw = New-Object System.IO.BinaryWriter($fs)
        try {
            $bw.Write([UInt16]0)
            $bw.Write([UInt16]1)
            $bw.Write([UInt16]1)
            $bw.Write([byte]($(if ($Width -ge 256) { 0 } else { $Width })))
            $bw.Write([byte]($(if ($Height -ge 256) { 0 } else { $Height })))
            $bw.Write([byte]0)
            $bw.Write([byte]0)
            $bw.Write([UInt16]1)
            $bw.Write([UInt16]32)
            $bw.Write([UInt32]$PngBytes.Length)
            $bw.Write([UInt32]22)
            $bw.Write($PngBytes)
        }
        finally {
            $bw.Close()
        }
    }
    finally {
        $fs.Close()
    }
}

function Convert-PngToIco {
    param(
        [Parameter(Mandatory = $true)] [string] $SourcePng,
        [Parameter(Mandatory = $true)] [string] $OutputIco
    )

    Add-Type -AssemblyName System.Drawing

    $srcBytes = [System.IO.File]::ReadAllBytes($SourcePng)
    $srcStream = New-Object System.IO.MemoryStream(, $srcBytes)
    try {
        $image = [System.Drawing.Image]::FromStream($srcStream)
        try {
            if ($image.Width -ne $image.Height) {
                throw "PNG 图标必须为正方形，当前尺寸 $($image.Width)x$($image.Height)"
            }

            Write-PngIco -PngBytes $srcBytes -OutputPath $OutputIco -Width $image.Width -Height $image.Height
        }
        finally {
            $image.Dispose()
        }
    }
    finally {
        $srcStream.Dispose()
    }
}

if ((Test-Path $uiSourcePng) -and (Test-Path $coreSourcePng)) {
    $needsUiIcon = -not (Test-Path $uiIconIco) -or ((Get-Item $uiIconIco).LastWriteTime -lt (Get-Item $uiSourcePng).LastWriteTime)
    $needsCoreIcon = -not (Test-Path $coreIconIco) -or ((Get-Item $coreIconIco).LastWriteTime -lt (Get-Item $coreSourcePng).LastWriteTime)

    if ($needsUiIcon) {
        Write-Host "Generating UI icon file..." -ForegroundColor Yellow
        Convert-PngToIco -SourcePng $uiSourcePng -OutputIco $uiIconIco
    }

    if ($needsCoreIcon) {
        Write-Host "Generating Core icon file..." -ForegroundColor Yellow
        Convert-PngToIco -SourcePng $coreSourcePng -OutputIco $coreIconIco
    }

    $needsGoResource = -not (Test-Path $goSyso) -or ((Get-Item $goSyso).LastWriteTime -lt (Get-Item $uiIconIco).LastWriteTime)
    if ($needsGoResource) {
        Write-Host "Generating Go Windows icon resource..." -ForegroundColor Yellow
        Push-Location "$PSScriptRoot\..\go-ui"
        go run github.com/akavel/rsrc@latest -arch amd64 -ico "..\assets\app-icon-ui.ico" -o "rsrc_windows_amd64.syso"
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Failed to generate Go icon resource!" -ForegroundColor Red
            Pop-Location
            exit 1
        }
        Pop-Location
    }
}

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

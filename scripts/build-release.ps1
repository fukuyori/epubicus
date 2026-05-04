# Build the release executable used for direct Windows runs:
#   .\target\release\epubicus.exe

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $ProjectRoot
try {
    cargo build --release
    $exe = Join-Path $ProjectRoot "target\release\epubicus.exe"
    Write-Host "Built $exe"
} finally {
    Pop-Location
}

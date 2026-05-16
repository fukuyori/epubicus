# Build the release executable used for direct Windows runs:
#   .\target\release\epubicus.exe

param(
    [Alias("h")]
    [switch]$Help
)

if ($Help) {
    foreach ($line in (Get-Content -LiteralPath $PSCommandPath)) {
        if ($line -match '^#') { ($line -replace '^#\s?', '') }
        else { break }
    }
    exit 0
}

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

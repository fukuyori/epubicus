# Write manual translations directly into the newest DeepSeek recovery cache.
#
# Usage:
#   .\scripts\manual-recover-deepseek.ps1 .\book.epub .\book.manual.json
#   .\scripts\manual-recover-deepseek.ps1 .\book.epub .\book.manual.json -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$ManualPath,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ManualPath)) {
    throw "Manual translation JSON path is required."
}

$script = Join-Path $PSScriptRoot "recover-from-cache.ps1"
& $script -InputPath $InputPath -Provider deepseek -Manual $ManualPath -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

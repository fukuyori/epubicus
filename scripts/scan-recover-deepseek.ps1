# Scan the standard DeepSeek output EPUB and recover suspicious blocks.
#
# Usage:
#   .\scripts\scan-recover-deepseek.ps1 .\book.epub
#   .\scripts\scan-recover-deepseek.ps1 .\book.epub -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$script = Join-Path $PSScriptRoot "scan-and-recover.ps1"
& $script -InputPath $InputPath -Provider deepseek -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

# Recover untranslated blocks from the newest DeepSeek recovery log.
#
# Usage:
#   .\scripts\recover-deepseek.ps1 .\book.epub
#   .\scripts\recover-deepseek.ps1 .\book.epub deepseek-v4-pro
#   .\scripts\recover-deepseek.ps1 .\book.epub deepseek-v4-pro -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$Model = "deepseek-v4-flash",

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$script = Join-Path $PSScriptRoot "recover-from-cache.ps1"
& $script -InputPath $InputPath -Provider deepseek -Model $Model -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

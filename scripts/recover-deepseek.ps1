# Recover untranslated blocks from the newest DeepSeek recovery log.
#
# Usage:
#   .\scripts\recover-deepseek.ps1 .\book.epub
#   .\scripts\recover-deepseek.ps1 .\book.epub deepseek-v4-pro
#   .\scripts\recover-deepseek.ps1 .\book.epub -Reason invalid_cached_translation -Limit 50
#   .\scripts\recover-deepseek.ps1 .\book.epub deepseek-v4-pro -Concurrency 2 -NoRun
#   .\scripts\recover-deepseek.ps1 .\book.epub deepseek-v4-pro -DevBuild -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$Model = "deepseek-v4-flash",

    [int]$Concurrency = 2,

    [int]$Limit = 0,

    [string[]]$Reason = @(),

    [switch]$DevBuild,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$script = Join-Path $PSScriptRoot "recover-from-cache.ps1"
& $script -InputPath $InputPath -Provider deepseek -Model $Model -Concurrency $Concurrency -Limit $Limit -Reason $Reason -DevBuild:$DevBuild -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

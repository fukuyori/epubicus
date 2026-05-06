# Rebuild a DeepSeek output EPUB from the existing cache.
#
# Layout:
#   auto   = use epubicus automatic Kindle fixed-layout detection
#   fixed  = force Kindle fixed-layout metadata
#   reflow = suppress Kindle fixed-layout metadata
#
# Usage:
#   .\scripts\rebuild-deepseek.ps1 .\book.epub
#   .\scripts\rebuild-deepseek.ps1 .\book.epub fixed
#   .\scripts\rebuild-deepseek.ps1 .\book.epub fixed -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [ValidateSet("auto", "fixed", "reflow")]
    [string]$Layout = "auto",

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$envScript = Join-Path $PSScriptRoot "deepseek-env.ps1"
if (-not (Test-Path -LiteralPath $envScript -PathType Leaf)) {
    $envScript = Join-Path $PSScriptRoot "deepseek-env.template.ps1"
}

& $envScript `
    -InputPath $InputPath `
    -PartialFromCache `
    -KindleFixedLayout:($Layout -eq "fixed") `
    -NoKindleFixedLayout:($Layout -eq "reflow") `
    -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

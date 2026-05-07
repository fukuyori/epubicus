# Inspect an EPUB and print its table of contents.
#
# Usage:
#   .\scripts\inspect-epub.ps1 .\book.epub
#   .\scripts\inspect-epub.ps1 .\book.epub -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}
$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path

function Invoke-EpubicusStep {
    param(
        [string]$Name,
        [string[]]$StepArgs
    )
    Write-Host ""
    Write-Host "[$Name]"
    if (-not $NoRun) {
        cargo run --release --quiet -- @StepArgs
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }
}

Write-Host ""
Write-Host "InputEpub = $InputEpub"

Invoke-EpubicusStep "inspect" @("inspect", $InputEpub)
Invoke-EpubicusStep "toc" @("toc", $InputEpub)

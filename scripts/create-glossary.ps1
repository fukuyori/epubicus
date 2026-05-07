# Create glossary candidate files next to an EPUB.
#
# Given:
#   D:\books\sample.epub
#
# This writes:
#   D:\books\sample.json
#   D:\books\sample.md
#
# Usage:
#   .\scripts\create-glossary.ps1 .\test\sample.epub
#   .\scripts\create-glossary.ps1 .\test\sample.epub -DevBuild
#   .\scripts\create-glossary.ps1 .\test\sample.epub -Force
#   .\scripts\create-glossary.ps1 .\test\sample.epub -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [int]$MinOccurrences = 3,

    [int]$MaxEntries = 200,

    [string]$EpubicusExe,

    [switch]$DevBuild,

    [switch]$Force,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}

$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($InputEpub)

$OutputJson = Join-Path $inputDir "$inputBaseName.json"
$OutputMarkdown = Join-Path $inputDir "$inputBaseName.md"

if (-not $Force) {
    $existing = @()
    if (Test-Path -LiteralPath $OutputJson -PathType Leaf) {
        $existing += $OutputJson
    }
    if (Test-Path -LiteralPath $OutputMarkdown -PathType Leaf) {
        $existing += $OutputMarkdown
    }
    if ($existing.Count -gt 0) {
        Write-Warning "Output file already exists; leaving it unchanged. Use -Force to overwrite: $($existing -join ', ')"
        exit 0
    }
}

$args = @(
    "glossary",
    $InputEpub,
    "--output", $OutputJson,
    "--review-prompt", $OutputMarkdown,
    "--min-occurrences", "$MinOccurrences",
    "--max-entries", "$MaxEntries"
)

Write-Host ""
Write-Host "InputEpub = $InputEpub"
Write-Host "JSON      = $OutputJson"
Write-Host "Markdown  = $OutputMarkdown"
Write-Host ""

if (-not [string]::IsNullOrWhiteSpace($EpubicusExe)) {
    $EpubicusExe = (Resolve-Path -LiteralPath $EpubicusExe).Path
    if (-not $NoRun) {
        & $EpubicusExe @args
        exit $LASTEXITCODE
    }
} else {
    if (-not $NoRun) {
        if ($DevBuild) {
            cargo run --quiet -- @args
        } else {
            cargo run --release --quiet -- @args
        }
        exit $LASTEXITCODE
    }
}

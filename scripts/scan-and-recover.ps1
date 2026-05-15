# Scan a translated EPUB for untranslated-looking blocks and optionally recover.
#
# Usage (-Provider is required):
#   .\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek            # scan + recover
#   .\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek -ScanOnly  # scan only (no API call)
#   .\scripts\scan-and-recover.ps1 .\book.epub -Provider claude
#   .\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub -Provider deepseek -NoRun
#   .\scripts\scan-and-recover.ps1 .\book.epub -Provider deepseek -KindleFixedLayout

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$OutputPath,

    [Parameter(Mandatory = $true)]
    [Alias("p")]
    [ValidateSet("ollama", "openai", "claude", "deepseek")]
    [string]$Provider,

    [Alias("m")]
    [string]$Model,

    [Alias("cr")]
    [string]$CacheRoot,

    [Alias("g")]
    [string]$Glossary,

    [Alias("l")]
    [int]$Limit = 0,

    [Alias("ListOnly")]
    [switch]$ScanOnly,

    [switch]$NoRebuild,

    [switch]$KindleFixedLayout,

    [switch]$NoKindleFixedLayout,

    [string]$EpubicusExe,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

if ($KindleFixedLayout -and $NoKindleFixedLayout) {
    throw "-KindleFixedLayout and -NoKindleFixedLayout cannot be used together."
}

$ProjectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}
$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($InputEpub)
$inputExtension = [System.IO.Path]::GetExtension($InputEpub)

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
}
$OutputEpub = (Resolve-Path -LiteralPath $OutputPath).Path

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".cache"
}
if (Test-Path -LiteralPath $CacheRoot) {
    $CacheRoot = (Resolve-Path -LiteralPath $CacheRoot).Path
}

if ([string]::IsNullOrWhiteSpace($Model)) {
    $Model = switch ($Provider) {
        "openai" { "gpt-5-mini" }
        "claude" { "claude-sonnet-4-5" }
        "deepseek" { "deepseek-v4-flash" }
        default { "qwen3:14b" }
    }
}

$GlossaryPath = $null
if (-not [string]::IsNullOrWhiteSpace($Glossary)) {
    $GlossaryPath = (Resolve-Path -LiteralPath $Glossary).Path
} else {
    $candidateGlossary = Join-Path $inputDir "$inputBaseName.json"
    if (Test-Path -LiteralPath $candidateGlossary -PathType Leaf) {
        $GlossaryPath = (Resolve-Path -LiteralPath $candidateGlossary).Path
    }
}

function Resolve-EpubicusExe {
    param([string]$Preferred)
    if (-not [string]::IsNullOrWhiteSpace($Preferred)) {
        return (Resolve-Path -LiteralPath $Preferred).Path
    }
    return $null
}

$args = @(
    "scan-recovery",
    $InputEpub,
    $OutputEpub,
    "--provider", $Provider,
    "--model", $Model,
    "--cache-root", $CacheRoot
)
if ($Limit -gt 0) {
    $args += @("--limit", "$Limit")
}
if (-not $ScanOnly) {
    $args += "--recover"
    if (-not $NoRebuild) {
        $args += "--rebuild"
    }
}
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    $args += @("--glossary", $GlossaryPath)
}
if ($KindleFixedLayout) {
    $args += "--kindle-fixed-layout"
}
if ($NoKindleFixedLayout) {
    $args += "--no-kindle-fixed-layout"
}

$exe = Resolve-EpubicusExe $EpubicusExe

Write-Host ""
Write-Host "InputEpub  = $InputEpub"
Write-Host "OutputEpub = $OutputEpub"
Write-Host "CacheRoot  = $CacheRoot"
Write-Host "Provider   = $Provider"
Write-Host "Model      = $Model"
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    Write-Host "Glossary   = $GlossaryPath"
}
Write-Host ""

if ($null -ne $exe) {
    if (-not $NoRun) {
        & $exe @args
        exit $LASTEXITCODE
    }
} else {
    if (-not $NoRun) {
        cargo run --release --quiet -- @args
        exit $LASTEXITCODE
    }
}

# Advanced recovery helper. Prefer provider-specific wrappers such as
# recover-deepseek.ps1 and manual-recover-deepseek.ps1 for daily use.
#
# Recover blocks from the newest recovery log for an input EPUB.
#
# This is useful after normal translate / partial-from-cache runs when epubicus
# printed a Recovery log path, but you want to avoid locating it manually.
#
# Usage:
#   .\scripts\recover-from-cache.ps1 .\book.epub
#   .\scripts\recover-from-cache.ps1 .\book.epub -CacheRoot .\.local-ollama-cache
#   .\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -Manual .\book.manual.json
#   .\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [ValidateSet("ollama", "openai", "claude", "deepseek")]
    [string]$Provider = "ollama",

    [string]$Model,

    [string]$CacheRoot,

    [string]$Glossary,

    [string]$Manual,

    [int]$Limit = 0,

    [int]$Page = 0,

    [int]$Block = 0,

    [string[]]$Reason = @(),

    [string]$Output,

    [switch]$NoRebuild,

    [switch]$KindleFixedLayout,

    [switch]$NoKindleFixedLayout,

    [switch]$List,

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

function Resolve-EpubicusExe {
    param([string]$Preferred)
    if (-not [string]::IsNullOrWhiteSpace($Preferred)) {
        return (Resolve-Path -LiteralPath $Preferred).Path
    }
    return $null
}

function Resolve-CacheRoot {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

if ([string]::IsNullOrWhiteSpace($CacheRoot) -and $Provider -eq "deepseek") {
    $CacheRoot = Join-Path $ProjectRoot ".deepseek-cache"
}
$ResolvedCacheRoot = Resolve-CacheRoot $CacheRoot

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

$ManualPath = $null
if (-not [string]::IsNullOrWhiteSpace($Manual)) {
    $ManualPath = (Resolve-Path -LiteralPath $Manual).Path
}

$args = @(
    "recover",
    "--cache", $InputEpub,
    "--provider", $Provider,
    "--model", $Model
)
if (-not [string]::IsNullOrWhiteSpace($ResolvedCacheRoot)) {
    $args += @("--cache-root", $ResolvedCacheRoot)
}
if ($Limit -gt 0) {
    $args += @("--limit", "$Limit")
}
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    $args += @("--glossary", $GlossaryPath)
}
if (-not [string]::IsNullOrWhiteSpace($ManualPath)) {
    $args += @("--manual", $ManualPath)
}
if ($Page -gt 0) {
    $args += @("--page", "$Page")
}
if ($Block -gt 0) {
    $args += @("--block", "$Block")
}
foreach ($reasonValue in $Reason) {
    if (-not [string]::IsNullOrWhiteSpace($reasonValue)) {
        $args += @("--reason", $reasonValue)
    }
}
if ($List) {
    $args += "--list"
} elseif (-not $NoRebuild) {
    $args += "--rebuild"
}
if (-not [string]::IsNullOrWhiteSpace($Output)) {
    $args += @("--output", $Output)
}
if ($KindleFixedLayout) {
    $args += "--kindle-fixed-layout"
}
if ($NoKindleFixedLayout) {
    $args += "--no-kindle-fixed-layout"
}

$exe = Resolve-EpubicusExe $EpubicusExe

Write-Host ""
Write-Host "InputEpub = $InputEpub"
if (-not [string]::IsNullOrWhiteSpace($ResolvedCacheRoot)) {
    Write-Host "CacheRoot = $ResolvedCacheRoot"
} else {
    Write-Host "CacheRoot = OS default"
}
Write-Host "Provider  = $Provider"
Write-Host "Model     = $Model"
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    Write-Host "Glossary  = $GlossaryPath"
}
if (-not [string]::IsNullOrWhiteSpace($ManualPath)) {
    Write-Host "Manual    = $ManualPath"
}
Write-Host ""

if ($null -ne $exe) {
    Write-Host "$exe $($args -join ' ')"
    if (-not $NoRun) {
        & $exe @args
        exit $LASTEXITCODE
    }
} else {
    Write-Host "cargo run --release -- $($args -join ' ')"
    if (-not $NoRun) {
        cargo run --release -- @args
        exit $LASTEXITCODE
    }
}

# Provider-agnostic recovery script. Reads the newest recovery log for an input
# EPUB and re-translates failed blocks via the chosen provider (or skips API
# calls when --Manual is given to write a hand-edited JSON into the cache).
#
# This is the body script used after a normal translate / partial-from-cache run
# emits a Recovery log path.
#
# Usage:
#   .\scripts\recover-from-cache.ps1 .\book.epub -Provider deepseek
#   .\scripts\recover-from-cache.ps1 .\book.epub -p deepseek -m deepseek-v4-pro
#   .\scripts\recover-from-cache.ps1 .\book.epub -p deepseek -Manual .\book.manual.json
#   .\scripts\recover-from-cache.ps1 .\book.epub -p openai -CacheRoot .\.cache
#   .\scripts\recover-from-cache.ps1 .\book.epub -p deepseek -List
#   .\scripts\recover-from-cache.ps1 .\book.epub -p deepseek -DevBuild -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Alias("p")]
    [ValidateSet("ollama", "openai", "claude", "deepseek")]
    [string]$Provider = "ollama",

    [Alias("m")]
    [string]$Model,

    [Alias("c")]
    [int]$Concurrency = 0,

    [Alias("cr")]
    [string]$CacheRoot,

    [Alias("g")]
    [string]$Glossary,

    [Alias("mn")]
    [string]$Manual,

    [Alias("l")]
    [int]$Limit = 0,

    [int]$Page = 0,

    [int]$Block = 0,

    [string[]]$Reason = @(),

    [string]$Output,

    [switch]$NoRebuild,

    [switch]$KindleFixedLayout,

    [switch]$NoKindleFixedLayout,

    [switch]$List,

    [switch]$DevBuild,

    [switch]$NoRun,

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

function Resolve-CacheRoot {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    if (Test-Path -LiteralPath $Path) {
        return (Resolve-Path -LiteralPath $Path).Path
    }
    # Cache directory may not exist yet on first use; pass through so epubicus creates it.
    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return Join-Path (Get-Location).Path $Path
}

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".cache"
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
if ($Concurrency -gt 0) {
    $args += @("--concurrency", "$Concurrency")
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

$OutputEpub = if (-not [string]::IsNullOrWhiteSpace($Output)) {
    $Output
} else {
    Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
}

Write-Host ""
Write-Host "InputEpub  = $InputEpub"
if (-not $List -and -not $NoRebuild) {
    Write-Host "OutputEpub = $OutputEpub"
}
if (-not [string]::IsNullOrWhiteSpace($ResolvedCacheRoot)) {
    Write-Host "CacheRoot  = $ResolvedCacheRoot"
} else {
    Write-Host "CacheRoot  = OS default"
}
Write-Host "Model      = $Model"
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    Write-Host "Glossary   = $GlossaryPath"
}
if (-not [string]::IsNullOrWhiteSpace($ManualPath)) {
    Write-Host "Manual     = $ManualPath"
}
Write-Host ""

Write-Host "Recovery:"
if (-not $NoRun) {
    if ($DevBuild) {
        cargo run --quiet -- @args
    } else {
        cargo run --release --quiet -- @args
    }
    exit $LASTEXITCODE
}

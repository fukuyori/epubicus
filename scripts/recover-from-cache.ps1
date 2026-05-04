# Recover blocks from the newest recovery log for an input EPUB.
#
# This is useful after normal translate / partial-from-cache runs when epubicus
# printed a Recovery log path, but you want to avoid locating it manually.
#
# Usage:
#   .\scripts\recover-from-cache.ps1 .\book.epub
#   .\scripts\recover-from-cache.ps1 .\book.epub -CacheRoot .\.local-ollama-cache
#   .\scripts\recover-from-cache.ps1 .\book.epub -Provider ollama -Model qwen3:14b -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [ValidateSet("ollama", "openai", "claude")]
    [string]$Provider = "ollama",

    [string]$Model = "qwen3:14b",

    [string]$CacheRoot,

    [int]$Limit = 0,

    [int]$Page = 0,

    [int]$Block = 0,

    [string[]]$Reason = @(),

    [string]$Output,

    [switch]$NoRebuild,

    [switch]$List,

    [string]$EpubicusExe,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}
$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path

function Resolve-EpubicusExe {
    param([string]$Preferred)
    if (-not [string]::IsNullOrWhiteSpace($Preferred)) {
        return (Resolve-Path -LiteralPath $Preferred).Path
    }
    $debugExe = Join-Path $ProjectRoot "target\debug\epubicus.exe"
    if (Test-Path -LiteralPath $debugExe -PathType Leaf) {
        return (Resolve-Path -LiteralPath $debugExe).Path
    }
    $releaseExe = Join-Path $ProjectRoot "target\release\epubicus.exe"
    if (Test-Path -LiteralPath $releaseExe -PathType Leaf) {
        return (Resolve-Path -LiteralPath $releaseExe).Path
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

$ResolvedCacheRoot = Resolve-CacheRoot $CacheRoot

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
Write-Host ""

if ($null -ne $exe) {
    Write-Host "$exe $($args -join ' ')"
    if (-not $NoRun) {
        & $exe @args
        exit $LASTEXITCODE
    }
} else {
    Write-Host "cargo run -- $($args -join ' ')"
    if (-not $NoRun) {
        cargo run -- @args
        exit $LASTEXITCODE
    }
}

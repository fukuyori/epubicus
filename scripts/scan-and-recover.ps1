# Scan a translated EPUB for untranslated-looking blocks and optionally recover.
#
# Usage:
#   .\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub -NoRun
#   .\scripts\scan-and-recover.ps1 .\book.epub .\book_jp.epub -Provider ollama -Model qwen3:14b

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$OutputPath,

    [ValidateSet("ollama", "openai", "claude")]
    [string]$Provider = "ollama",

    [string]$Model = "qwen3:14b",

    [string]$CacheRoot,

    [string]$Glossary,

    [int]$Limit = 0,

    [Alias("ListOnly")]
    [switch]$ScanOnly,

    [switch]$NoRebuild,

    [string]$EpubicusExe,

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
$inputExtension = [System.IO.Path]::GetExtension($InputEpub)

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
}
$OutputEpub = (Resolve-Path -LiteralPath $OutputPath).Path

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = switch ($Provider) {
        "openai" { Join-Path $ProjectRoot ".openai-cache" }
        "claude" { Join-Path $ProjectRoot ".claude-cache" }
        default { Join-Path $ProjectRoot ".local-ollama-cache" }
    }
}
if (Test-Path -LiteralPath $CacheRoot) {
    $CacheRoot = (Resolve-Path -LiteralPath $CacheRoot).Path
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

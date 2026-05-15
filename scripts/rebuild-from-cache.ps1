# Rebuild an output EPUB from the existing cache without calling the provider API.
#
# Provider/model are auto-detected from the cache (the dominant combination found
# in <cache>/<input_hash>/translations.jsonl). Override with -Provider / -Model.
#
# Usage:
#   .\scripts\rebuild.ps1 .\book.epub
#   .\scripts\rebuild.ps1 .\book.epub fixed
#   .\scripts\rebuild.ps1 .\book.epub reflow
#   .\scripts\rebuild.ps1 .\book.epub -Provider claude
#
# Layout:
#   auto   = use epubicus automatic Kindle fixed-layout detection
#   fixed  = force Kindle fixed-layout metadata
#   reflow = suppress Kindle fixed-layout metadata

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [ValidateSet("auto", "fixed", "reflow")]
    [string]$Layout = "auto",

    [Alias("p")]
    [ValidateSet("ollama", "openai", "claude", "deepseek")]
    [string]$Provider,

    [Alias("m")]
    [string]$Model,

    [Alias("cr")]
    [string]$CacheRoot,

    [Alias("g")]
    [string]$Glossary,

    [switch]$DevBuild,

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
$OutputEpub = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".cache"
}

# Auto-detect provider/model from existing cache when not given explicitly.
if ([string]::IsNullOrWhiteSpace($Provider) -or [string]::IsNullOrWhiteSpace($Model)) {
    $fullHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $InputEpub).Hash.ToLower()
    $inputHash = $fullHash.Substring(0, 32)
    $translationsFile = Join-Path $CacheRoot $inputHash "translations.jsonl"

    if (-not (Test-Path -LiteralPath $translationsFile)) {
        throw "No cache found at $translationsFile. Run translate first or specify both -Provider and -Model."
    }

    $entries = Get-Content -LiteralPath $translationsFile -Encoding UTF8 | ForEach-Object {
        try { $_ | ConvertFrom-Json -ErrorAction Stop } catch { $null }
    } | Where-Object { $_ -ne $null -and $_.provider -and $_.model }

    if ($entries.Count -eq 0) {
        throw "Cache file has no usable entries: $translationsFile"
    }

    $dominant = $entries |
        Group-Object -Property { "$($_.provider)|$($_.model)" } |
        Sort-Object Count -Descending |
        Select-Object -First 1
    $parts = $dominant.Name -split '\|', 2

    if ([string]::IsNullOrWhiteSpace($Provider)) { $Provider = $parts[0] }
    if ([string]::IsNullOrWhiteSpace($Model))    { $Model    = $parts[1] }

    Write-Host "Detected from cache: Provider=$Provider, Model=$Model ($($dominant.Count)/$($entries.Count) entries)"
}

# Glossary auto-detect.
$GlossaryPath = $null
if (-not [string]::IsNullOrWhiteSpace($Glossary)) {
    $GlossaryPath = (Resolve-Path -LiteralPath $Glossary).Path
} else {
    $candidateGlossary = Join-Path $inputDir "$inputBaseName.json"
    if (Test-Path -LiteralPath $candidateGlossary -PathType Leaf) {
        $GlossaryPath = (Resolve-Path -LiteralPath $candidateGlossary).Path
    }
}

$args = @(
    "translate",
    $InputEpub,
    "--provider", $Provider,
    "--model", $Model,
    "--cache-root", $CacheRoot,
    "--keep-cache",
    "--output", $OutputEpub,
    "--partial-from-cache"
)
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    $args += @("--glossary", $GlossaryPath)
}
if ($Layout -eq "fixed") {
    $args += "--kindle-fixed-layout"
}
if ($Layout -eq "reflow") {
    $args += "--no-kindle-fixed-layout"
}

Write-Host ""
Write-Host "InputEpub  = $InputEpub"
Write-Host "OutputEpub = $OutputEpub"
Write-Host "CacheRoot  = $CacheRoot"
Write-Host "Provider   = $Provider"
Write-Host "Model      = $Model"
Write-Host "Layout     = $Layout"
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    Write-Host "Glossary   = $GlossaryPath"
}
Write-Host ""

if (-not $NoRun) {
    if ($DevBuild) {
        cargo run --quiet -- @args
    } else {
        cargo run --release --quiet -- @args
    }
    exit $LASTEXITCODE
}

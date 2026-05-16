# epubicus Ollama (local) translate script.
#
# Usage:
#   Run a normal local conversion:
#     .\scripts\translate-ollama.ps1 .\test\sample.epub
#
#   Page-range test:
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -From 3 -To 3
#
#   Usage estimate only (no API call):
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -From 3 -To 3 -UsageOnly
#
#   Rebuild from cache without calling the provider:
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -PartialFromCache
#
#   Kindle fixed-layout output (force or suppress):
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -KindleFixedLayout
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -NoKindleFixedLayout
#
#   Pass additional epubicus translate options:
#     .\scripts\translate-ollama.ps1 .\test\sample.epub --glossary .\glossary.json
#
#   Or load it without running:
#     . .\scripts\translate-ollama.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusTranslate
#
#   Use Cargo dev profile for script/debug checks:
#     .\scripts\translate-ollama.ps1 .\test\sample.epub -DevBuild -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Alias("f")]
    [int]$From = 0,

    [Alias("t")]
    [int]$To = 0,

    [Alias("m")]
    [string]$Model = "qwen3:14b",

    [Alias("c")]
    [int]$Concurrency = 3,

    [Alias("s")]
    [string]$Style = "essay",

    [string[]]$ExtraArgs = @(),

    [switch]$UsageOnly,

    [switch]$PartialFromCache,

    [switch]$KindleFixedLayout,

    [switch]$NoKindleFixedLayout,

    [switch]$DevBuild,

    [switch]$NoRun,

    [Alias("h")]
    [switch]$Help,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassthroughArgs = @()
)

if ($Help) {
    foreach ($line in (Get-Content -LiteralPath $PSCommandPath)) {
        if ($line -match '^#') { ($line -replace '^#\s?', '') }
        else { break }
    }
    exit 0
}

if ($KindleFixedLayout -and $NoKindleFixedLayout) {
    throw "-KindleFixedLayout and -NoKindleFixedLayout cannot be used together."
}

$ProjectRoot = Split-Path -Parent $PSScriptRoot

$defaultInput = Join-Path $ProjectRoot "test\sample.epub"
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = $defaultInput
}

$global:InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $global:InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($global:InputEpub)
$inputExtension = [System.IO.Path]::GetExtension($global:InputEpub)
$global:OutputEpub = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
$global:CacheRoot = Join-Path $ProjectRoot ".cache"
$ExtraArgs = @($ExtraArgs) + @($PassthroughArgs)
$global:GlossaryPath = $null
if (($ExtraArgs -notcontains "--glossary") -and ($ExtraArgs -notcontains "-g") -and -not ($ExtraArgs | Where-Object { $_.StartsWith("--glossary=") })) {
    $candidateGlossary = Join-Path $inputDir "$inputBaseName.json"
    if (Test-Path -LiteralPath $candidateGlossary -PathType Leaf) {
        $global:GlossaryPath = (Resolve-Path -LiteralPath $candidateGlossary).Path
    }
}

$env:EPUBICUS_PROVIDER = "ollama"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_OLLAMA_HOST = "http://localhost:11434"
$env:EPUBICUS_STYLE = $Style
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_NUM_CTX = "8192"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "$Concurrency"
$env:EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE = "true"

# No API key required for local Ollama.

function New-EpubicusTranslateArgs {
    $args = @(
        "translate",
        $global:InputEpub,
        "--cache-root", $global:CacheRoot,
        "--keep-cache",
        "--output", $global:OutputEpub
    )
    if (-not [string]::IsNullOrWhiteSpace($global:GlossaryPath)) {
        $args += @("--glossary", $global:GlossaryPath)
    }
    if ($From -gt 0) {
        $args += @("--from", "$From")
    }
    if ($To -gt 0) {
        $args += @("--to", "$To")
    }
    if ($UsageOnly) {
        $args += "--usage-only"
    }
    if ($PartialFromCache) {
        $args += "--partial-from-cache"
    }
    if ($KindleFixedLayout) {
        $args += "--kindle-fixed-layout"
    }
    if ($NoKindleFixedLayout) {
        $args += "--no-kindle-fixed-layout"
    }
    $args += $ExtraArgs
    return $args
}

function Show-EpubicusTranslateCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host "Provider   = $env:EPUBICUS_PROVIDER"
    Write-Host "Model      = $env:EPUBICUS_MODEL"
    if (-not [string]::IsNullOrWhiteSpace($global:GlossaryPath)) {
        Write-Host "Glossary   = $global:GlossaryPath"
    }
    if ($ExtraArgs.Count -gt 0) {
        Write-Host "ExtraArgs  = $($ExtraArgs -join ' ')"
    }
    Write-Host ""
    Write-Host "Conversion:"
    Write-Host "Invoke-EpubicusTranslate"
    Write-Host ""
}

function Invoke-EpubicusTranslate {
    if ($DevBuild) {
        cargo run --quiet -- @(New-EpubicusTranslateArgs)
    } else {
        cargo run --release --quiet -- @(New-EpubicusTranslateArgs)
    }
}

Show-EpubicusTranslateCommands

if (-not $NoRun) {
    Invoke-EpubicusTranslate
}

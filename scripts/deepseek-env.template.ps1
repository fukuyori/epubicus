# epubicus DeepSeek normal API environment template.
#
# Usage:
#   Copy this file to a local name if you want to customize it:
#     Copy-Item .\scripts\deepseek-env.template.ps1 .\scripts\deepseek-env.ps1
#
#   Run a normal DeepSeek API conversion:
#     .\scripts\deepseek-env.ps1 .\test\sample.epub
#
#   Page-range test:
#     .\scripts\deepseek-env.ps1 .\test\sample.epub -From 3 -To 3
#
#   Rebuild from cache without calling the provider:
#     .\scripts\deepseek-env.ps1 .\test\sample.epub -PartialFromCache
#
#   Pass additional epubicus translate options:
#     .\scripts\deepseek-env.ps1 .\test\sample.epub -ExtraArgs @("--glossary", ".\glossary.json")
#     .\scripts\deepseek-env.ps1 .\test\sample.epub --glossary .\glossary.json
#
#   Or load it without running:
#     . .\scripts\deepseek-env.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusDeepSeek

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [int]$From = 0,

    [int]$To = 0,

    [string]$Model = "deepseek-v4-flash",

    [int]$Concurrency = 2,

    [string[]]$ExtraArgs = @(),

    [switch]$UsageOnly,

    [switch]$PartialFromCache,

    [switch]$KindleFixedLayout,

    [switch]$NoKindleFixedLayout,

    [switch]$NoRun,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassthroughArgs = @()
)

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
$global:CacheRoot = Join-Path $ProjectRoot ".deepseek-cache"
$ExtraArgs = @($ExtraArgs) + @($PassthroughArgs)
$global:GlossaryPath = $null
if (($ExtraArgs -notcontains "--glossary") -and ($ExtraArgs -notcontains "-g") -and -not ($ExtraArgs | Where-Object { $_.StartsWith("--glossary=") })) {
    $candidateGlossary = Join-Path $inputDir "$inputBaseName.json"
    if (Test-Path -LiteralPath $candidateGlossary -PathType Leaf) {
        $global:GlossaryPath = (Resolve-Path -LiteralPath $candidateGlossary).Path
    }
}

$env:EPUBICUS_PROVIDER = "deepseek"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "$Concurrency"
$env:EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE = "true"

if ((-not $UsageOnly) -and (-not $NoRun) -and [string]::IsNullOrWhiteSpace($env:DEEPSEEK_API_KEY)) {
    $env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput
}

if ((-not $UsageOnly) -and (-not $NoRun) -and [string]::IsNullOrWhiteSpace($env:DEEPSEEK_API_KEY)) {
    Write-Warning "DEEPSEEK_API_KEY is not set. Set it before running DeepSeek API commands:"
    Write-Warning '$env:DEEPSEEK_API_KEY = Read-Host "DeepSeek API key" -MaskInput'
}

function New-EpubicusDeepSeekArgs {
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

function Show-EpubicusDeepSeekCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host "Model      = $env:EPUBICUS_MODEL"
    if (-not [string]::IsNullOrWhiteSpace($global:GlossaryPath)) {
        Write-Host "Glossary   = $global:GlossaryPath"
    }
    if ($ExtraArgs.Count -gt 0) {
        Write-Host "ExtraArgs  = $($ExtraArgs -join ' ')"
    }
    Write-Host ""
    Write-Host "Normal DeepSeek conversion:"
    Write-Host "Invoke-EpubicusDeepSeek"
    Write-Host "cargo run --release -- $((New-EpubicusDeepSeekArgs) -join ' ')"
    Write-Host ""
}

function Invoke-EpubicusDeepSeek {
    cargo run --release -- @(New-EpubicusDeepSeekArgs)
}

Show-EpubicusDeepSeekCommands

if (-not $NoRun) {
    Invoke-EpubicusDeepSeek
}

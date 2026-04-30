# epubicus OpenAI normal API environment template.
#
# Usage:
#   Copy this file to a local name if you want to customize it:
#     Copy-Item .\scripts\openai-env.template.ps1 .\scripts\openai-env.ps1
#
#   Run a normal OpenAI API conversion:
#     .\scripts\openai-env.ps1 .\test\sample.epub
#
#   Page-range test:
#     .\scripts\openai-env.ps1 .\test\sample.epub -From 3 -To 3
#
#   Or load it without running:
#     . .\scripts\openai-env.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusOpenAi

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [int]$From = 0,

    [int]$To = 0,

    [string]$Model = "gpt-5-mini",

    [int]$Concurrency = 4,

    [switch]$UsageOnly,

    [switch]$NoRun
)

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
$global:CacheRoot = Join-Path $ProjectRoot ".openai-cache"

$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_OPENAI_BASE_URL = "https://api.openai.com/v1"
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "$Concurrency"

if ([string]::IsNullOrWhiteSpace($env:OPENAI_API_KEY)) {
    Write-Warning "OPENAI_API_KEY is not set. Set it before running OpenAI API commands:"
    Write-Warning '$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput'
}

function New-EpubicusOpenAiArgs {
    $args = @(
        "translate",
        $global:InputEpub,
        "--cache-root", $global:CacheRoot,
        "--keep-cache",
        "--output", $global:OutputEpub
    )
    if ($From -gt 0) {
        $args += @("--from", "$From")
    }
    if ($To -gt 0) {
        $args += @("--to", "$To")
    }
    if ($UsageOnly) {
        $args += "--usage-only"
    }
    return $args
}

function Show-EpubicusOpenAiCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host "Model      = $env:EPUBICUS_MODEL"
    Write-Host ""
    Write-Host "Normal OpenAI conversion:"
    Write-Host "Invoke-EpubicusOpenAi"
    Write-Host "cargo run -- $((New-EpubicusOpenAiArgs) -join ' ')"
    Write-Host ""
}

function Invoke-EpubicusOpenAi {
    cargo run -- @(New-EpubicusOpenAiArgs)
}

Show-EpubicusOpenAiCommands

if (-not $NoRun) {
    Invoke-EpubicusOpenAi
}

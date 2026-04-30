# epubicus OpenAI Batch API environment template.
#
# Usage:
#   Copy this file to a local name if you want to customize it:
#     Copy-Item .\scripts\openai-batch-env.template.ps1 .\scripts\openai-batch-env.ps1
#
#   Run a Batch API conversion:
#     .\scripts\openai-batch-env.ps1 .\test\sample.epub
#
#   Page-range test:
#     .\scripts\openai-batch-env.ps1 .\test\sample.epub -From 3 -To 3
#
#   Or load it without running, then call a helper command:
#     . .\scripts\openai-batch-env.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusOpenAiBatch
#     Invoke-EpubicusOpenAiBatchStatus
#     Invoke-EpubicusOpenAiBatchVerify

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [int]$From = 0,

    [int]$To = 0,

    [string]$Model = "gpt-5-mini",

    [int]$PollSecs = 60,

    [switch]$NoWait,

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
$global:CacheRoot = Join-Path $ProjectRoot ".batch-openai-cache"

$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_OPENAI_BASE_URL = "https://api.openai.com/v1"
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "1"

if ([string]::IsNullOrWhiteSpace($env:OPENAI_API_KEY)) {
    Write-Warning "OPENAI_API_KEY is not set. Set it before running Batch API commands:"
    Write-Warning '$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput'
}

function New-EpubicusBatchArgs {
    $args = @(
        "run",
        $global:InputEpub,
        "--provider", "openai",
        "--model", $env:EPUBICUS_MODEL,
        "--cache-root", $global:CacheRoot,
        "--force-prepare",
        "--poll-secs", "$PollSecs",
        "--output", $global:OutputEpub
    )
    if (-not $NoWait) {
        $args += "--wait"
    }
    if ($From -gt 0) {
        $args += @("--from", "$From")
    }
    if ($To -gt 0) {
        $args += @("--to", "$To")
    }
    return $args
}

function Show-EpubicusOpenAiBatchCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host "Model      = $env:EPUBICUS_MODEL"
    Write-Host ""
    Write-Host "Batch conversion:"
    Write-Host "Invoke-EpubicusOpenAiBatch"
    Write-Host "cargo run -- batch $((New-EpubicusBatchArgs) -join ' ')"
    Write-Host ""
    Write-Host "Status:"
    Write-Host "Invoke-EpubicusOpenAiBatchStatus"
    Write-Host ""
    Write-Host "Verify:"
    Write-Host "Invoke-EpubicusOpenAiBatchVerify"
    Write-Host ""
}

function Invoke-EpubicusOpenAiBatch {
    cargo run -- batch @(New-EpubicusBatchArgs)
}

function Invoke-EpubicusOpenAiBatchStatus {
    cargo run -- batch status $global:InputEpub --cache-root $global:CacheRoot
}

function Invoke-EpubicusOpenAiBatchVerify {
    cargo run -- batch verify $global:InputEpub --cache-root $global:CacheRoot
}

Show-EpubicusOpenAiBatchCommands

if (-not $NoRun) {
    Invoke-EpubicusOpenAiBatch
}

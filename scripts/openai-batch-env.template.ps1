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
#   Use a glossary:
#     .\scripts\openai-batch-env.ps1 .\test\sample.epub -Glossary .\glossary.json
#     .\scripts\openai-batch-env.ps1 .\test\sample.epub --glossary .\glossary.json
#
#   Pass additional epubicus batch run options:
#     .\scripts\openai-batch-env.ps1 .\test\sample.epub -ExtraArgs @("--max-wait-secs", "3600")
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

    [string]$Glossary,

    [string[]]$ExtraArgs = @(),

    [int]$PollSecs = 60,

    [switch]$NoWait,

    [switch]$NoRun,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassthroughArgs = @()
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
$global:GlossaryPath = $null
if (-not [string]::IsNullOrWhiteSpace($Glossary)) {
    $global:GlossaryPath = (Resolve-Path -LiteralPath $Glossary).Path
}
$ExtraArgs = @($ExtraArgs) + @($PassthroughArgs)

$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_OPENAI_BASE_URL = "https://api.openai.com/v1"
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "1"
$env:EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE = "true"

if ([string]::IsNullOrWhiteSpace($env:OPENAI_API_KEY)) {
    Write-Warning "OPENAI_API_KEY is not set. Set it before running Batch API commands:"
    Write-Warning '$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput'
}

function New-EpubicusBatchCommonArgs {
    $args = @(
        $global:InputEpub,
        "--cache-root", $global:CacheRoot
    )
    if (-not [string]::IsNullOrWhiteSpace($global:GlossaryPath)) {
        $args += @("--glossary", $global:GlossaryPath)
    }
    return $args
}

function New-EpubicusBatchArgs {
    $args = @("run") + (New-EpubicusBatchCommonArgs) + @(
        "--provider", "openai",
        "--model", $env:EPUBICUS_MODEL,
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
    $args += $ExtraArgs
    return $args
}

function Show-EpubicusOpenAiBatchCommands {
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
    cargo run -- batch status @(New-EpubicusBatchCommonArgs)
}

function Invoke-EpubicusOpenAiBatchVerify {
    cargo run -- batch verify @(New-EpubicusBatchCommonArgs)
}

Show-EpubicusOpenAiBatchCommands

if (-not $NoRun) {
    Invoke-EpubicusOpenAiBatch
}

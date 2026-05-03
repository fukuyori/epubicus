# epubicus Claude normal API environment template.
#
# Usage:
#   Copy this file to a local name if you want to customize it:
#     Copy-Item .\scripts\claude-env.template.ps1 .\scripts\claude-env.ps1
#
#   Run a normal Claude API conversion:
#     .\scripts\claude-env.ps1 .\test\sample.epub
#
#   Page-range test:
#     .\scripts\claude-env.ps1 .\test\sample.epub -From 3 -To 3
#
#   Pass additional epubicus translate options:
#     .\scripts\claude-env.ps1 .\test\sample.epub -ExtraArgs @("--glossary", ".\glossary.json")
#     .\scripts\claude-env.ps1 .\test\sample.epub --glossary .\glossary.json
#
#   Or load it without running:
#     . .\scripts\claude-env.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusClaude

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [int]$From = 0,

    [int]$To = 0,

    [string]$Model = "claude-sonnet-4-5",

    [int]$Concurrency = 1,

    [string[]]$ExtraArgs = @(),

    [switch]$UsageOnly,

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
$global:CacheRoot = Join-Path $ProjectRoot ".claude-cache"
$ExtraArgs = @($ExtraArgs) + @($PassthroughArgs)

$env:EPUBICUS_PROVIDER = "claude"
$env:EPUBICUS_MODEL = $Model
$env:EPUBICUS_CLAUDE_BASE_URL = "https://api.anthropic.com/v1"
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_CONCURRENCY = "$Concurrency"
$env:EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE = "true"

if ([string]::IsNullOrWhiteSpace($env:ANTHROPIC_API_KEY)) {
    Write-Warning "ANTHROPIC_API_KEY is not set. Set it before running Claude API commands:"
    Write-Warning '$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput'
}

function New-EpubicusClaudeArgs {
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
    $args += $ExtraArgs
    return $args
}

function Show-EpubicusClaudeCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host "Model      = $env:EPUBICUS_MODEL"
    if ($ExtraArgs.Count -gt 0) {
        Write-Host "ExtraArgs  = $($ExtraArgs -join ' ')"
    }
    Write-Host ""
    Write-Host "Normal Claude conversion:"
    Write-Host "Invoke-EpubicusClaude"
    Write-Host "cargo run --release -- $((New-EpubicusClaudeArgs) -join ' ')"
    Write-Host ""
}

function Invoke-EpubicusClaude {
    cargo run --release -- @(New-EpubicusClaudeArgs)
}

Show-EpubicusClaudeCommands

if (-not $NoRun) {
    Invoke-EpubicusClaude
}


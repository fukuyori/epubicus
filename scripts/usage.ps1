# Estimate API usage for a selected content-file range without calling the provider.
# Works for any supported provider (ollama, openai, claude, deepseek).
#
# Usage:
#   .\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek
#   .\scripts\usage.ps1 .\book.epub 9 9 -Provider claude
#   .\scripts\usage.ps1 .\book.epub 9 9 -Provider openai
#   .\scripts\usage.ps1 .\book.epub 9 9 -Provider ollama
#   .\scripts\usage.ps1 .\book.epub 9 9 -Provider deepseek -Model deepseek-v4-pro
#
# Arguments:
#   1: input EPUB path.
#   2: first 1-based OPF spine number. Required.
#   3: last 1-based OPF spine number. Required.
#   -Provider: required. One of ollama / openai / claude / deepseek.
#
# This command does not call the provider API. It only prints estimated request
# counts and input/output tokens for the selected range.

[CmdletBinding(DefaultParameterSetName = 'Run')]
param(
    [Parameter(Position = 0, ParameterSetName = 'Run')]
    [string]$InputPath,

    [Parameter(Position = 1, ParameterSetName = 'Run')]
    [int]$From = 0,

    [Parameter(Position = 2, ParameterSetName = 'Run')]
    [int]$To = 0,

    [Parameter(Mandatory = $true, ParameterSetName = 'Run')]
    [Alias("p")]
    [ValidateSet("ollama", "openai", "claude", "deepseek")]
    [string]$Provider,

    [Parameter(ParameterSetName = 'Run')]
    [Alias("m")]
    [string]$Model,

    [Parameter(ParameterSetName = 'Run')]
    [Alias("cr")]
    [string]$CacheRoot,

    [Parameter(ParameterSetName = 'Run')]
    [Alias("g")]
    [string]$Glossary,

    [Parameter(ParameterSetName = 'Run')]
    [switch]$DevBuild,

    [Parameter(ParameterSetName = 'Run')]
    [switch]$NoRun,

    [Parameter(Mandatory = $true, ParameterSetName = 'Help')]
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

if ($From -le 0 -or $To -le 0) {
    throw "Start and end spine numbers are required. Run .\scripts\inspect-epub.ps1 first, then pass a range such as: .\scripts\usage.ps1 .\book.epub 5 5 -Provider deepseek"
}

$ProjectRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}
$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($InputEpub)

# Provider-specific defaults (mirrors the env templates).
if ([string]::IsNullOrWhiteSpace($Model)) {
    $Model = switch ($Provider) {
        "ollama"   { "qwen3:14b" }
        "openai"   { "gpt-5-mini" }
        "claude"   { "claude-sonnet-4-5" }
        "deepseek" { "deepseek-v4-flash" }
    }
}

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".cache"
}

# Auto-detect glossary next to the EPUB.
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
    "--from", "$From",
    "--to", "$To",
    "--usage-only"
)
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    $args += @("--glossary", $GlossaryPath)
}

Write-Host ""
Write-Host "InputEpub  = $InputEpub"
Write-Host "Provider   = $Provider"
Write-Host "Model      = $Model"
Write-Host "CacheRoot  = $CacheRoot"
Write-Host "Range      = $From..$To"
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

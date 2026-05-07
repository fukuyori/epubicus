# Recover untranslated blocks from the newest recovery log using OpenAI.
#
# This wrapper is useful after a DeepSeek run leaves recoverable validation
# failures: it keeps the existing cache and writes successful OpenAI
# translations back to the original recovery cache keys.
#
# Usage:
#   .\scripts\recover-openai.ps1 .\book.epub
#   .\scripts\recover-openai.ps1 .\book.epub gpt-5-mini -Concurrency 4 -NoRun
#   .\scripts\recover-openai.ps1 .\book.epub gpt-5-mini -DevBuild -NoRun
#   .\scripts\recover-openai.ps1 .\book.epub -Limit 20 -NoRebuild

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [string]$Model = "gpt-5-mini",

    [int]$Concurrency = 4,

    [string]$CacheRoot,

    [string]$Glossary,

    [int]$Limit = 0,

    [int]$Page = 0,

    [int]$Block = 0,

    [string[]]$Reason = @(),

    [string]$Output,

    [switch]$NoRebuild,

    [switch]$List,

    [switch]$DevBuild,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".deepseek-cache"
}

$script = Join-Path $PSScriptRoot "recover-from-cache.ps1"
$recoverParams = @{
    InputPath = $InputPath
    Provider = "openai"
    Model = $Model
    Concurrency = $Concurrency
    CacheRoot = $CacheRoot
}
if ($NoRebuild) {
    $recoverParams.NoRebuild = $true
}
if ($List) {
    $recoverParams.List = $true
}
if ($NoRun) {
    $recoverParams.NoRun = $true
}
if ($DevBuild) {
    $recoverParams.DevBuild = $true
}

if (-not [string]::IsNullOrWhiteSpace($Glossary)) {
    $recoverParams.Glossary = $Glossary
}
if ($Limit -gt 0) {
    $recoverParams.Limit = $Limit
}
if ($Page -gt 0) {
    $recoverParams.Page = $Page
}
if ($Block -gt 0) {
    $recoverParams.Block = $Block
}
$reasonValues = @($Reason | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
if ($reasonValues.Count -gt 0) {
    $recoverParams.Reason = $reasonValues
}
if (-not [string]::IsNullOrWhiteSpace($Output)) {
    $recoverParams.Output = $Output
}
& $script @recoverParams
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

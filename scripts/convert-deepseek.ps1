# Run a normal DeepSeek conversion with the standard project defaults.
#
# Usage:
#   .\scripts\convert-deepseek.ps1 .\book.epub
#   .\scripts\convert-deepseek.ps1 .\book.epub --style novel
#   .\scripts\convert-deepseek.ps1 .\book.epub -NoRun

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [switch]$NoRun,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassthroughArgs = @()
)

$ErrorActionPreference = "Stop"

$envScript = Join-Path $PSScriptRoot "deepseek-env.ps1"
if (-not (Test-Path -LiteralPath $envScript -PathType Leaf)) {
    $envScript = Join-Path $PSScriptRoot "deepseek-env.template.ps1"
}

& $envScript -InputPath $InputPath -NoRun:$NoRun @PassthroughArgs
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

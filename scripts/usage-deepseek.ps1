# Estimate DeepSeek API usage for a selected content-file range without calling the provider.
#
# Usage:
#   .\scripts\usage-deepseek.ps1 .\book.epub 9 9
#   .\scripts\usage-deepseek.ps1 .\book.epub 9 9 -NoRun
#
# Arguments:
#   1: input EPUB path.
#   2: first 1-based OPF spine number. Required.
#   3: last 1-based OPF spine number. Required.
#
# The second and third arguments are not reader page numbers.

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [Parameter(Position = 1)]
    [int]$From = 0,

    [Parameter(Position = 2)]
    [int]$To = 0,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

if ($From -le 0 -or $To -le 0) {
    throw "Start and end spine numbers are required. Run .\scripts\inspect-epub.ps1 first, then pass a range such as: .\scripts\usage-deepseek.ps1 .\book.epub 5 5"
}

$envScript = Join-Path $PSScriptRoot "deepseek-env.ps1"
if (-not (Test-Path -LiteralPath $envScript -PathType Leaf)) {
    $envScript = Join-Path $PSScriptRoot "deepseek-env.template.ps1"
}

& $envScript -InputPath $InputPath -From $From -To $To -UsageOnly -NoRun:$NoRun
if ($global:LASTEXITCODE -is [int]) {
    exit $LASTEXITCODE
}

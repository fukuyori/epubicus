# epubicus local Ollama environment template.
#
# Usage:
#   Copy this file to a local name if you want to customize it:
#     Copy-Item .\scripts\local-ollama-env.template.ps1 .\scripts\local-ollama-env.ps1
#
#   Run a full local conversion:
#     .\scripts\local-ollama-env.ps1 .\test\sample.epub
#
#   Or load it without running, then call a helper command:
#     . .\scripts\local-ollama-env.ps1 .\test\sample.epub -NoRun
#     Invoke-EpubicusLocalPageCheck
#     Invoke-EpubicusLocalFull
#     Invoke-EpubicusAssembleFromCache
#
#   Pass additional epubicus translate options:
#     .\scripts\local-ollama-env.ps1 .\test\sample.epub -ExtraArgs @("--glossary", ".\glossary.json")
#     .\scripts\local-ollama-env.ps1 .\test\sample.epub --glossary .\glossary.json

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [ValidateSet("full", "page", "cache")]
    [string]$Mode = "full",

    [int]$From = 3,

    [int]$To = 3,

    [string[]]$ExtraArgs = @(),

    [switch]$NoRun,

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$PassthroughArgs = @()
)

$ProjectRoot = Split-Path -Parent $PSScriptRoot

# Input/output paths used by the helper commands below.
$defaultInput = Join-Path $ProjectRoot "test\sample.epub"
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = $defaultInput
}

$global:InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $global:InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($global:InputEpub)
$inputExtension = [System.IO.Path]::GetExtension($global:InputEpub)
$global:OutputEpub = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
$global:CacheRoot = Join-Path $ProjectRoot ".local-ollama-cache"
$ExtraArgs = @($ExtraArgs) + @($PassthroughArgs)

# Local provider defaults.
$env:EPUBICUS_PROVIDER = "ollama"
$env:EPUBICUS_MODEL = "qwen3:14b"
$env:EPUBICUS_OLLAMA_HOST = "http://localhost:11434"

# Translation behavior.
$env:EPUBICUS_STYLE = "essay"
$env:EPUBICUS_TEMPERATURE = "0.3"
$env:EPUBICUS_NUM_CTX = "8192"
$env:EPUBICUS_TIMEOUT_SECS = "900"
$env:EPUBICUS_RETRIES = "3"
$env:EPUBICUS_MAX_CHARS_PER_REQUEST = "3500"
$env:EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE = "true"

# Ollama is usually safest at 1 while validating output quality.
# Increase only if the local model/server has enough headroom.
$env:EPUBICUS_CONCURRENCY = "2"

function New-EpubicusLocalTranslateArgs {
    param(
        [switch]$PartialFromCache,
        [int]$From = 0,
        [int]$To = 0
    )

    $args = @(
        "translate",
        $global:InputEpub,
        "--cache-root", $global:CacheRoot,
        "--keep-cache",
        "--output", $global:OutputEpub
    )
    if ($PartialFromCache) {
        $args += "--partial-from-cache"
    } else {
        $args += "--passthrough-on-validation-failure"
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

function Show-EpubicusLocalCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    if ($ExtraArgs.Count -gt 0) {
        Write-Host "ExtraArgs  = $($ExtraArgs -join ' ')"
    }
    Write-Host ""
    Write-Host "Local page-range check:"
    Write-Host "Invoke-EpubicusLocalPageCheck"
    Write-Host "cargo run -- $((New-EpubicusLocalTranslateArgs -From $From -To $To) -join ' ')"
    Write-Host ""
    Write-Host "Local full conversion:"
    Write-Host "Invoke-EpubicusLocalFull"
    Write-Host "cargo run -- $((New-EpubicusLocalTranslateArgs) -join ' ')"
    Write-Host ""
    Write-Host "Assemble from cache only:"
    Write-Host "Invoke-EpubicusAssembleFromCache"
    Write-Host "cargo run -- $((New-EpubicusLocalTranslateArgs -PartialFromCache) -join ' ')"
    Write-Host ""
}

function Invoke-EpubicusLocalPageCheck {
    param(
        [int]$From = 3,
        [int]$To = 3
    )

    cargo run -- @(New-EpubicusLocalTranslateArgs -From $From -To $To)
}

function Invoke-EpubicusLocalFull {
    cargo run -- @(New-EpubicusLocalTranslateArgs)
}

function Invoke-EpubicusAssembleFromCache {
    cargo run -- @(New-EpubicusLocalTranslateArgs -PartialFromCache)
}

Show-EpubicusLocalCommands

if (-not $NoRun) {
    switch ($Mode) {
        "page" {
            Invoke-EpubicusLocalPageCheck -From $From -To $To
        }
        "cache" {
            Invoke-EpubicusAssembleFromCache
        }
        default {
            Invoke-EpubicusLocalFull
        }
    }
}

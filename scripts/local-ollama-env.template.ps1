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

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [ValidateSet("full", "page", "cache")]
    [string]$Mode = "full",

    [int]$From = 3,

    [int]$To = 3,

    [switch]$NoRun
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

# Ollama is usually safest at 1 while validating output quality.
# Increase only if the local model/server has enough headroom.
$env:EPUBICUS_CONCURRENCY = "2"

# Optional API settings. Leave unset for local-only runs.
# $env:OPENAI_API_KEY = "sk-..."
# $env:EPUBICUS_PROVIDER = "openai"
# $env:EPUBICUS_MODEL = "gpt-5-mini"

function Show-EpubicusLocalCommands {
    Write-Host ""
    Write-Host "InputEpub  = $global:InputEpub"
    Write-Host "OutputEpub = $global:OutputEpub"
    Write-Host "CacheRoot  = $global:CacheRoot"
    Write-Host ""
    Write-Host "Local page-range check:"
    Write-Host "Invoke-EpubicusLocalPageCheck"
    Write-Host "cargo run -- translate `$InputEpub --cache-root `$CacheRoot --from 3 --to 3 --keep-cache --output `$OutputEpub"
    Write-Host ""
    Write-Host "Local full conversion:"
    Write-Host "Invoke-EpubicusLocalFull"
    Write-Host "cargo run -- translate `$InputEpub --cache-root `$CacheRoot --keep-cache --output `$OutputEpub"
    Write-Host ""
    Write-Host "Assemble from cache only:"
    Write-Host "Invoke-EpubicusAssembleFromCache"
    Write-Host "cargo run -- translate `$InputEpub --cache-root `$CacheRoot --partial-from-cache --keep-cache --output `$OutputEpub"
    Write-Host ""
}

function Invoke-EpubicusLocalPageCheck {
    param(
        [int]$From = 3,
        [int]$To = 3
    )

    cargo run -- translate $global:InputEpub `
        --cache-root $global:CacheRoot `
        --from $From `
        --to $To `
        --keep-cache `
        --output $global:OutputEpub
}

function Invoke-EpubicusLocalFull {
    cargo run -- translate $global:InputEpub `
        --cache-root $global:CacheRoot `
        --keep-cache `
        --output $global:OutputEpub
}

function Invoke-EpubicusAssembleFromCache {
    cargo run -- translate $global:InputEpub `
        --cache-root $global:CacheRoot `
        --partial-from-cache `
        --keep-cache `
        --output $global:OutputEpub
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

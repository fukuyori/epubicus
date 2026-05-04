# Recover unfinished OpenAI Batch work locally, then rebuild the EPUB.
#
# The script uses .batch-openai-cache by default and does not stop running
# epubicus processes. Use -NoRun first when checking a command sequence.
#
# Usage:
#   .\scripts\batch-recover-local.ps1 .\book.epub -NoRun
#   .\scripts\batch-recover-local.ps1 .\book.epub -LocalModel qwen3:14b -Limit 100
#   .\scripts\batch-recover-local.ps1 .\book.epub -StrictVerify

param(
    [Parameter(Position = 0)]
    [string]$InputPath,

    [string]$CacheRoot,

    [string]$Output,

    [string]$Glossary,

    [string]$BatchModel = "gpt-5-mini",

    [ValidateSet("ollama", "openai", "claude")]
    [string]$LocalProvider = "ollama",

    [string]$LocalModel = "qwen3:14b",

    [int]$Limit = 100,

    [ValidateSet("page-order", "failed-first", "hard-first", "short-first", "oldest-first")]
    [string]$Priority = "short-first",

    [switch]$SkipFetchImport,

    [switch]$SkipLocal,

    [switch]$SkipRebuild,

    [switch]$StrictVerify,

    [string]$EpubicusExe,

    [switch]$NoRun
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($InputPath)) {
    $InputPath = Join-Path $ProjectRoot "test\sample.epub"
}
$InputEpub = (Resolve-Path -LiteralPath $InputPath).Path
$inputDir = Split-Path -Parent $InputEpub
$inputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($InputEpub)
$inputExtension = [System.IO.Path]::GetExtension($InputEpub)

if ([string]::IsNullOrWhiteSpace($CacheRoot)) {
    $CacheRoot = Join-Path $ProjectRoot ".batch-openai-cache"
}
$CacheRoot = (Resolve-Path -LiteralPath $CacheRoot).Path

if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path $inputDir "$inputBaseName`_jp$inputExtension"
}

$GlossaryPath = $null
if (-not [string]::IsNullOrWhiteSpace($Glossary)) {
    $GlossaryPath = (Resolve-Path -LiteralPath $Glossary).Path
} else {
    $candidateGlossary = Join-Path $inputDir "$inputBaseName.json"
    if (Test-Path -LiteralPath $candidateGlossary -PathType Leaf) {
        $GlossaryPath = (Resolve-Path -LiteralPath $candidateGlossary).Path
    }
}

function Resolve-EpubicusExe {
    param([string]$Preferred)
    if (-not [string]::IsNullOrWhiteSpace($Preferred)) {
        return (Resolve-Path -LiteralPath $Preferred).Path
    }
    $debugExe = Join-Path $ProjectRoot "target\debug\epubicus.exe"
    if (Test-Path -LiteralPath $debugExe -PathType Leaf) {
        return (Resolve-Path -LiteralPath $debugExe).Path
    }
    $releaseExe = Join-Path $ProjectRoot "target\release\epubicus.exe"
    if (Test-Path -LiteralPath $releaseExe -PathType Leaf) {
        return (Resolve-Path -LiteralPath $releaseExe).Path
    }
    return $null
}

function New-CommonArgs {
    $common = @($InputEpub, "--cache-root", $CacheRoot)
    if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
        $common += @("--glossary", $GlossaryPath)
    }
    return $common
}

function Invoke-Step {
    param(
        [string]$Name,
        [string[]]$StepArgs,
        [switch]$ContinueOnError
    )
    Write-Host ""
    Write-Host "[$Name]"
    if ($null -ne $script:Epubicus) {
        Write-Host "$script:Epubicus $($StepArgs -join ' ')"
        if (-not $NoRun) {
            & $script:Epubicus @StepArgs
            if ($LASTEXITCODE -ne 0) {
                if ($ContinueOnError) {
                    Write-Warning "$Name failed with exit code $LASTEXITCODE; continuing."
                    return
                }
                throw "$Name failed with exit code $LASTEXITCODE"
            }
        }
    } else {
        Write-Host "cargo run -- $($StepArgs -join ' ')"
        if (-not $NoRun) {
            cargo run -- @StepArgs
            if ($LASTEXITCODE -ne 0) {
                if ($ContinueOnError) {
                    Write-Warning "$Name failed with exit code $LASTEXITCODE; continuing."
                    return
                }
                throw "$Name failed with exit code $LASTEXITCODE"
            }
        }
    }
}

$script:Epubicus = Resolve-EpubicusExe $EpubicusExe

Write-Host ""
Write-Host "InputEpub     = $InputEpub"
Write-Host "OutputEpub    = $Output"
Write-Host "CacheRoot     = $CacheRoot"
Write-Host "BatchModel    = $BatchModel"
Write-Host "LocalProvider = $LocalProvider"
Write-Host "LocalModel    = $LocalModel"
if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) {
    Write-Host "Glossary      = $GlossaryPath"
}

$common = New-CommonArgs

Invoke-Step "health before" (@("batch", "health") + $common)

if (-not $SkipFetchImport) {
    Invoke-Step "fetch" (@("batch", "fetch") + $common)
    Invoke-Step "import" (@("batch", "import") + $common)
}

if (-not $SkipLocal) {
    Invoke-Step "reroute local" (@("batch", "reroute-local") + $common + @(
        "--provider", "openai",
        "--model", $BatchModel,
        "--remaining",
        "--priority", $Priority
    ))
    $translateLocal = @("batch", "translate-local") + $common + @(
        "--provider", $LocalProvider,
        "--model", $LocalModel,
        "--priority", $Priority
    )
    if ($Limit -gt 0) {
        $translateLocal += @("--limit", "$Limit")
    }
    Invoke-Step "translate local" $translateLocal
}

Invoke-Step "verify" (@("batch", "verify") + $common) -ContinueOnError:(-not $StrictVerify)
Invoke-Step "health after" (@("batch", "health") + $common)

if (-not $SkipRebuild) {
    Invoke-Step "rebuild epub" (@(
        "translate",
        $InputEpub,
        "--cache-root", $CacheRoot,
        "--provider", "openai",
        "--model", $BatchModel,
        "--partial-from-cache",
        "--keep-cache",
        "--output", $Output
    ) + $(if (-not [string]::IsNullOrWhiteSpace($GlossaryPath)) { @("--glossary", $GlossaryPath) } else { @() }))
}

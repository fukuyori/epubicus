# Clear all known epubicus cache roots.
#
# This removes both the OS-default cache and project-local cache roots used by
# the helper scripts. It does not stop running epubicus processes; locked files
# are reported and left for a later cleanup.
#
# Usage:
#   .\scripts\clear-all-caches.ps1 -DryRun
#   .\scripts\clear-all-caches.ps1 -Yes
#   .\scripts\clear-all-caches.ps1 -Include .\some-other-cache -Yes

param(
    [switch]$DryRun,
    [switch]$Yes,
    [string[]]$Include = @()
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot

function Resolve-ExistingPath {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    try {
        return (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
    } catch {
        return $null
    }
}

function Get-DefaultCacheRoot {
    if ($IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
            return Join-Path $env:LOCALAPPDATA "epubicus\cache"
        }
        if (-not [string]::IsNullOrWhiteSpace($env:APPDATA)) {
            return Join-Path $env:APPDATA "epubicus\cache"
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($env:XDG_CACHE_HOME)) {
        return Join-Path $env:XDG_CACHE_HOME "epubicus"
    }
    if (-not [string]::IsNullOrWhiteSpace($HOME)) {
        return Join-Path $HOME ".cache\epubicus"
    }
    return $null
}

$candidateRoots = @(
    (Get-DefaultCacheRoot),
    (Join-Path $ProjectRoot ".epubicus-cache"),
    (Join-Path $ProjectRoot ".openai-cache"),
    (Join-Path $ProjectRoot ".batch-openai-cache"),
    (Join-Path $ProjectRoot ".local-ollama-cache"),
    (Join-Path $ProjectRoot ".claude-cache")
)

foreach ($path in $Include) {
    $candidateRoots += $path
}

$roots = [System.Collections.Generic.List[string]]::new()
foreach ($path in $candidateRoots) {
    $resolved = Resolve-ExistingPath $path
    if ($null -eq $resolved) {
        continue
    }
    if (-not $roots.Contains($resolved)) {
        $roots.Add($resolved)
    }
}

if ($roots.Count -eq 0) {
    Write-Host "No epubicus cache roots found."
    exit 0
}

$items = foreach ($root in $roots) {
    $size = 0L
    $fileCount = 0
    Get-ChildItem -LiteralPath $root -Recurse -Force -File -ErrorAction SilentlyContinue | ForEach-Object {
        $size += $_.Length
        $fileCount += 1
    }
    [pscustomobject]@{
        Path = $root
        Files = $fileCount
        SizeMB = [math]::Round($size / 1MB, 2)
    }
}

Write-Host "epubicus cache roots:"
$items | Format-Table -AutoSize

if ($DryRun) {
    Write-Host "Dry run only. Nothing was deleted."
    exit 0
}

if (-not $Yes) {
    $answer = Read-Host "Delete all cache roots listed above? Type yes"
    if ($answer -ne "yes") {
        Write-Host "Cancelled."
        exit 1
    }
}

$deleted = 0
$failed = 0
foreach ($root in $roots) {
    try {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
        Write-Host "Deleted: $root"
        $deleted += 1
    } catch {
        Write-Warning "Failed to delete: $root"
        Write-Warning $_.Exception.Message
        $failed += 1
    }
}

Write-Host "Done. Deleted $deleted cache root(s); failed $failed."
if ($failed -gt 0) {
    exit 2
}

#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Build hw-helper when its assembly is missing or older than its sources.

.DESCRIPTION
  Cheap idempotent guard meant to run before `tauri dev`. Skips the (slow)
  `dotnet publish` when the helper is up to date so iterative dev launches
  stay fast. Use `build-helper.ps1` directly to force a rebuild.
#>

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$assembly = Join-Path $root "src-tauri\resources\hw-helper\hw-helper.dll"
$helperDir = Join-Path $root "hw-helper"

function Test-ModuleBundle {
    $directory = Split-Path $assembly
    foreach ($file in @("hw-helper.dll", "hw-helper.deps.json", "hw-helper.runtimeconfig.json", "hostfxr.dll", "LibreHardwareMonitorLib.dll")) {
        if (-not (Test-Path -LiteralPath (Join-Path $directory $file) -PathType Leaf)) { return $false }
    }
    try {
        $config = Get-Content -LiteralPath (Join-Path $directory "hw-helper.runtimeconfig.json") -Raw | ConvertFrom-Json
        $version = $config.runtimeOptions.framework.version
        return $version -match '^\d+\.\d+\.\d+$' -and (Test-Path -LiteralPath (Join-Path $directory "shared\Microsoft.NETCore.App\$version\coreclr.dll"))
    } catch { return $false }
}

function Get-LatestSourceWriteTime {
    $files = Get-ChildItem -LiteralPath $helperDir -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object {
            $_.Extension -in @(".cs", ".csproj") -and
            $_.FullName -notmatch "\\bin\\" -and
            $_.FullName -notmatch "\\obj\\" -and
            $_.FullName -notmatch "\\publish\\"
        }
    $latest = $files | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if ($latest) { return $latest.LastWriteTimeUtc }
    return [DateTime]::MinValue
}

if (Test-ModuleBundle) {
    $assemblyTime = (Get-Item -LiteralPath (Join-Path (Split-Path $assembly) "hw-helper.runtimeconfig.json")).LastWriteTimeUtc
    $sourceTime = Get-LatestSourceWriteTime
    foreach ($script in @($PSCommandPath, (Join-Path $PSScriptRoot "build-helper.ps1"))) {
        $time = (Get-Item -LiteralPath $script).LastWriteTimeUtc
        if ($time -gt $sourceTime) { $sourceTime = $time }
    }
    if ($assemblyTime -ge $sourceTime) {
        Write-Host ">>> hw-helper already up to date, skipping" -ForegroundColor DarkGray
        exit 0
    }
    Write-Host ">>> hw-helper sources changed; rebuilding..." -ForegroundColor Yellow
} else {
    Write-Host ">>> hw-helper.dll missing; running build-helper.ps1..." -ForegroundColor Yellow
}

& (Join-Path $PSScriptRoot "build-helper.ps1")
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
if (Test-ModuleBundle) {
    exit 0
}

Write-Host ">>> hw-helper build finished but assembly was not found: $assembly" -ForegroundColor Red
exit 1

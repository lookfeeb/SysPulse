#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Ensure src-tauri/resources/hw-helper/hw-helper.exe exists; build it if not.

.DESCRIPTION
  Cheap idempotent guard meant to run before `tauri dev`. Skips the (slow)
  `dotnet publish` when the helper is already on disk so iterative dev launches
  stay fast. Use `build-helper.ps1` directly to force a rebuild.
#>

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "src-tauri\resources\hw-helper\hw-helper.exe"
$helperDir = Join-Path $root "hw-helper"

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

if (Test-Path -LiteralPath $exe) {
    $exeTime = (Get-Item -LiteralPath $exe).LastWriteTimeUtc
    $sourceTime = Get-LatestSourceWriteTime
    if ($exeTime -ge $sourceTime) {
        Write-Host ">>> hw-helper already up to date, skipping" -ForegroundColor DarkGray
        exit 0
    }
    Write-Host ">>> hw-helper sources changed; rebuilding..." -ForegroundColor Yellow
} else {
    Write-Host ">>> hw-helper.exe missing; running build-helper.ps1..." -ForegroundColor Yellow
}

& (Join-Path $PSScriptRoot "build-helper.ps1")
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
if (Test-Path -LiteralPath $exe) {
    exit 0
}

Write-Host ">>> hw-helper build finished but exe was not found: $exe" -ForegroundColor Red
exit 1

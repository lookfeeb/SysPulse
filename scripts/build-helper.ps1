#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Build the C# hw-helper as a self-contained single-file Windows executable
  and copy the output into src-tauri/resources/hw-helper/.

.DESCRIPTION
  Run before `npm run tauri:dev` (first time) and before any `tauri:build`.
  Tauri will pick the contents of src-tauri/resources/hw-helper/ via the
  `bundle.resources` config.

.PARAMETER Configuration
  Release (default) or Debug.

.PARAMETER Runtime
  win-x64 (default) or win-arm64.
#>

param(
    [string]$Configuration = "Release",
    [string]$Runtime = "win-x64"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$helperDir = Join-Path $root "hw-helper"
$dest = Join-Path $root "src-tauri\resources\hw-helper"

Write-Host ">>> Publishing hw-helper ($Configuration / $Runtime)..." -ForegroundColor Cyan

Push-Location $helperDir
try {
    dotnet publish `
        -c $Configuration `
        -r $Runtime `
        --self-contained true `
        -p:PublishSingleFile=true `
        -p:PublishTrimmed=false `
        -p:IncludeNativeLibrariesForSelfExtract=true `
        -p:DebugType=embedded `
        -o (Join-Path $helperDir "publish")
    if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed" }
} finally {
    Pop-Location
}

# Wipe and re-copy.
if (Test-Path $dest) {
    Remove-Item -Recurse -Force $dest
}
New-Item -ItemType Directory -Path $dest | Out-Null

$publish = Join-Path $helperDir "publish"
Copy-Item -Path (Join-Path $publish "*") -Destination $dest -Recurse -Force

# We don't need the .pdb in production.
Get-ChildItem $dest -Filter *.pdb -Recurse | Remove-Item -Force

$size = (Get-ChildItem $dest -Recurse | Measure-Object -Property Length -Sum).Sum
$sizeMb = [math]::Round($size / 1MB, 1)
Write-Host ">>> hw-helper packaged at $dest ($sizeMb MB)" -ForegroundColor Green

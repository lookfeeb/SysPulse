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

if (Test-Path $exe) {
    Write-Host ">>> hw-helper already built, skipping (delete src-tauri\resources\hw-helper to force rebuild)" -ForegroundColor DarkGray
    exit 0
}

Write-Host ">>> hw-helper.exe missing; running build-helper.ps1..." -ForegroundColor Yellow
& (Join-Path $PSScriptRoot "build-helper.ps1")

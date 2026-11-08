#!/usr/bin/env pwsh
param([string]$Configuration = "Release", [ValidateSet("win-x64","win-arm64")][string]$Runtime = "win-x64")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$project = Join-Path $root "hw-helper\HwHelper.csproj"
$work = Join-Path $root "target\hw-inprocess-build"
$publish = Join-Path $work "publish"
$bundle = Join-Path $work "bundle"
$dest = Join-Path $root "src-tauri\resources\hw-helper"
function Clear-Generated([string]$path) {
    $absolute = [IO.Path]::GetFullPath($path)
    $workspace = [IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
    if (-not $absolute.StartsWith($workspace, [StringComparison]::OrdinalIgnoreCase)) { throw "Outside workspace: $absolute" }
    if (Test-Path -LiteralPath $absolute) {
        if ((Get-Item -LiteralPath $absolute).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing reparse point: $absolute" }
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
}
Clear-Generated $publish
Write-Host ">>> Resolving bundled .NET runtime ($Runtime)..."
dotnet publish $project -c $Configuration -r $Runtime --self-contained true -p:PublishSingleFile=false -p:UseAppHost=true -o $publish
if ($LASTEXITCODE -ne 0) { throw "Runtime restore/publish failed" }
$config = Get-Content -LiteralPath (Join-Path $publish "hw-helper.runtimeconfig.json") -Raw | ConvertFrom-Json
$runtimeVersion = ($config.runtimeOptions.includedFrameworks | Where-Object name -eq "Microsoft.NETCore.App").version
if (-not $runtimeVersion) { throw "Bundled runtime version missing" }
$nugetOutput = dotnet nuget locals global-packages --list
if ($LASTEXITCODE -ne 0) { throw "Cannot resolve NuGet cache" }
$packages = ($nugetOutput -replace '^global-packages:\s*', '').Trim()
$pack = Join-Path $packages "microsoft.netcore.app.runtime.$Runtime\$runtimeVersion\runtimes\$Runtime"
if (-not (Test-Path -LiteralPath (Join-Path $pack "native\hostfxr.dll"))) { throw "Missing .NET runtime pack: $pack" }
Clear-Generated $bundle
Write-Host ">>> Publishing in-process hardware assembly..."
dotnet publish $project -c $Configuration -r $Runtime --self-contained false -p:PublishSingleFile=false -p:UseAppHost=false -o $bundle
if ($LASTEXITCODE -ne 0) { throw "Managed publish failed" }
$framework = Join-Path $bundle "shared\Microsoft.NETCore.App\$runtimeVersion"
New-Item -ItemType Directory -Path $framework -Force | Out-Null
Copy-Item -Path (Join-Path $pack "native\*") -Destination $framework -Force
Copy-Item -Path (Join-Path $pack "lib\net8.0\*") -Destination $framework -Force
Copy-Item -LiteralPath (Join-Path $pack "native\hostfxr.dll") -Destination (Join-Path $bundle "hostfxr.dll") -Force
Get-ChildItem -LiteralPath $bundle -Filter *.pdb -Recurse | Remove-Item -Force
$config = Get-Content -LiteralPath (Join-Path $bundle "hw-helper.runtimeconfig.json") -Raw | ConvertFrom-Json
$config.runtimeOptions.framework.version = $runtimeVersion
$config | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $bundle "hw-helper.runtimeconfig.json") -Encoding utf8NoBOM
Clear-Generated $dest
New-Item -ItemType Directory -Path $dest -Force | Out-Null
Copy-Item -Path (Join-Path $bundle "*") -Destination $dest -Recurse -Force
Write-Host ">>> In-process hardware module and private .NET runtime packaged: $dest"

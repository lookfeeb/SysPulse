#!/usr/bin/env pwsh
<#
.SYNOPSIS
  SysPulse 交互式编译脚本

.DESCRIPTION
  1 - LTO 编译（完整优化，体积最小，速度最慢）
  2 - 普通编译（标准 Release，速度较快）
  3 - 清理缓存（cargo clean + node_modules/.cache + dist）
  0 - 退出

  编译完成后自动将安装包复制到 release/ 目录。
#>

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot

# ─── 配置区 ────────────────────────────────────────────────────────────────────

# 安装包输出目录（相对于项目根目录），用于存放准备发布的安装包
$releaseDir = Join-Path $root "release"

# Tauri 编译后安装包所在路径
$bundleDir = Join-Path $root "target\release\bundle"

# ─── 工具函数 ──────────────────────────────────────────────────────────────────

function Write-Header {
    Clear-Host
    $version = Get-ProjectVersion
    Write-Host ""
    Write-Host "  ╔══════════════════════════════════════╗" -ForegroundColor Cyan
    Write-Host "  ║        SysPulse 编译工具              ║" -ForegroundColor Cyan
    Write-Host "  ║        版本: $($version.PadRight(24))║" -ForegroundColor Cyan
    Write-Host "  ╚══════════════════════════════════════╝" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  [1] LTO 编译      (完整优化，体积最小)" -ForegroundColor White
    Write-Host "  [2] 普通编译      (标准 Release，速度较快)" -ForegroundColor White
    Write-Host "  [3] 清理缓存      (cargo clean + dist)" -ForegroundColor White
    Write-Host "  [0] 退出" -ForegroundColor DarkGray
    Write-Host ""
}

function Get-ProjectVersion {
    $tauriConf = Join-Path $root "src-tauri\tauri.conf.json"
    if (Test-Path $tauriConf) {
        $conf = Get-Content $tauriConf -Raw | ConvertFrom-Json
        return $conf.version
    }
    return "unknown"
}

function Set-ProjectVersion {
    param([string]$NewVersion)

    # 更新 tauri.conf.json
    $tauriConf = Join-Path $root "src-tauri\tauri.conf.json"
    $conf = Get-Content $tauriConf -Raw | ConvertFrom-Json
    $conf.version = $NewVersion
    $conf | ConvertTo-Json -Depth 20 | Set-Content $tauriConf -Encoding UTF8

    # 更新 package.json
    $pkgJson = Join-Path $root "package.json"
    $pkg = Get-Content $pkgJson -Raw | ConvertFrom-Json
    $pkg.version = $NewVersion
    $pkg | ConvertTo-Json -Depth 10 | Set-Content $pkgJson -Encoding UTF8

    # 更新 Cargo.toml — 只替换 [package] 段的 version，不动 dependencies
    $cargoToml = Join-Path $root "src-tauri\Cargo.toml"
    if (Test-Path $cargoToml) {
        $content = Get-Content $cargoToml -Raw
        # 匹配 [package] 到下一个 [section] 之间的 version = "x.y.z"
        $content = $content -replace '(?s)(\[package\].*?version\s*=\s*")[^"]*(")', "`${1}$NewVersion`${2}"
        Set-Content $cargoToml $content -Encoding UTF8
    }

    Write-Host "  ✓ 版本已更新为 $NewVersion" -ForegroundColor Green
}

function Ask-Version {
    $current = Get-ProjectVersion
    Write-Host ""
    Write-Host "  当前版本: $current" -ForegroundColor Yellow
    $input = Read-Host "  输入新版本号（直接回车保持不变）"
    $input = $input.Trim()
    if ($input -ne "" -and $input -ne $current) {
        Set-ProjectVersion $input
    }
}

function Build-Helper {
    Write-Host ""
    Write-Host "  ► 编译 hw-helper (C#)..." -ForegroundColor Cyan
    & (Join-Path $root "scripts\build-helper.ps1")
    if ($LASTEXITCODE -ne 0) { throw "hw-helper 编译失败" }
}

function Build-Frontend {
    Write-Host ""
    Write-Host "  ► 编译前端..." -ForegroundColor Cyan
    Push-Location $root
    try {
        npm run build
        if ($LASTEXITCODE -ne 0) { throw "前端编译失败" }
    } finally {
        Pop-Location
    }
}

function Copy-Installers {
    Write-Host ""
    Write-Host "  ► 复制安装包到 release/ ..." -ForegroundColor Cyan

    if (-not (Test-Path $releaseDir)) {
        New-Item -ItemType Directory -Path $releaseDir | Out-Null
    }

    $version = Get-ProjectVersion
    $copied = 0

    # 查找 NSIS (.exe) 和 MSI (.msi) 安装包
    $patterns = @("*.exe", "*.msi")
    foreach ($pattern in $patterns) {
        $files = Get-ChildItem -Path $bundleDir -Filter $pattern -Recurse -ErrorAction SilentlyContinue
        foreach ($file in $files) {
            # 跳过非安装包的 exe（如 uninstall）
            if ($file.Name -match "uninstall" -or $file.Name -match "Uninstall") { continue }

            $ext = $file.Extension
            $destName = "SysPulse_v${version}_setup${ext}"
            $destPath = Join-Path $releaseDir $destName
            Copy-Item $file.FullName $destPath -Force
            $sizeMb = [math]::Round($file.Length / 1MB, 1)
            Write-Host "    ✓ $destName ($sizeMb MB)" -ForegroundColor Green
            $copied++
        }
    }

    if ($copied -eq 0) {
        Write-Host "    ⚠ 未找到安装包，请检查 $bundleDir" -ForegroundColor Yellow
    } else {
        Write-Host ""
        Write-Host "  安装包已保存到: $releaseDir" -ForegroundColor Cyan
    }
}

function Show-Duration {
    param([System.Diagnostics.Stopwatch]$sw)
    $elapsed = $sw.Elapsed
    if ($elapsed.TotalMinutes -ge 1) {
        Write-Host "  耗时: $([int]$elapsed.TotalMinutes) 分 $($elapsed.Seconds) 秒" -ForegroundColor DarkGray
    } else {
        Write-Host "  耗时: $($elapsed.Seconds) 秒" -ForegroundColor DarkGray
    }
}

# ─── 编译动作 ──────────────────────────────────────────────────────────────────

function Invoke-LtoBuild {
    Ask-Version
    Write-Host ""
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Cyan
    Write-Host "  开始 LTO 编译（fat LTO + codegen-units=1）..." -ForegroundColor Cyan
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Cyan

    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    Build-Helper
    Build-Frontend

    Write-Host ""
    Write-Host "  ► 编译 Tauri (LTO)..." -ForegroundColor Cyan
    Push-Location $root
    try {
        # Cargo.toml [profile.release] 已有 lto=true，这里强制 fat 模式
        $env:CARGO_PROFILE_RELEASE_LTO = "fat"
        $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
        $env:CARGO_PROFILE_RELEASE_OPT_LEVEL = "z"

        npx tauri build
        if ($LASTEXITCODE -ne 0) { throw "Tauri LTO 编译失败" }
    } finally {
        Remove-Item Env:\CARGO_PROFILE_RELEASE_LTO -ErrorAction SilentlyContinue
        Remove-Item Env:\CARGO_PROFILE_RELEASE_CODEGEN_UNITS -ErrorAction SilentlyContinue
        Remove-Item Env:\CARGO_PROFILE_RELEASE_OPT_LEVEL -ErrorAction SilentlyContinue
        Pop-Location
    }

    $sw.Stop()
    Copy-Installers
    Write-Host ""
    Write-Host "  ✓ LTO 编译完成" -ForegroundColor Green
    Show-Duration $sw
}

function Invoke-NormalBuild {
    Ask-Version
    Write-Host ""
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Cyan
    Write-Host "  开始普通编译（关闭 LTO，速度更快）..." -ForegroundColor Cyan
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Cyan

    $sw = [System.Diagnostics.Stopwatch]::StartNew()

    Build-Helper
    Build-Frontend

    Write-Host ""
    Write-Host "  ► 编译 Tauri..." -ForegroundColor Cyan
    Push-Location $root
    try {
        # 关闭 LTO 以加快编译速度（覆盖 Cargo.toml 中的 lto=true）
        $env:CARGO_PROFILE_RELEASE_LTO = "false"
        $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16"

        npx tauri build
        if ($LASTEXITCODE -ne 0) { throw "Tauri 编译失败" }
    } finally {
        Remove-Item Env:\CARGO_PROFILE_RELEASE_LTO -ErrorAction SilentlyContinue
        Remove-Item Env:\CARGO_PROFILE_RELEASE_CODEGEN_UNITS -ErrorAction SilentlyContinue
        Pop-Location
    }

    $sw.Stop()
    Copy-Installers
    Write-Host ""
    Write-Host "  ✓ 普通编译完成" -ForegroundColor Green
    Show-Duration $sw
}

function Invoke-Clean {
    Write-Host ""
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Yellow
    Write-Host "  清理缓存..." -ForegroundColor Yellow
    Write-Host "  ════════════════════════════════════════" -ForegroundColor Yellow

    # cargo clean
    Write-Host "  ► cargo clean..." -ForegroundColor White
    Push-Location (Join-Path $root "src-tauri")
    try {
        cargo clean
    } finally {
        Pop-Location
    }

    # dist
    $distPath = Join-Path $root "dist"
    if (Test-Path $distPath) {
        Write-Host "  ► 清理 dist/..." -ForegroundColor White
        Remove-Item -Recurse -Force $distPath
    }

    # node_modules/.cache (vite cache)
    $viteCache = Join-Path $root "node_modules\.vite"
    if (Test-Path $viteCache) {
        Write-Host "  ► 清理 vite 缓存..." -ForegroundColor White
        Remove-Item -Recurse -Force $viteCache
    }

    # hw-helper publish 目录
    $helperPublish = Join-Path $root "hw-helper\publish"
    if (Test-Path $helperPublish) {
        Write-Host "  ► 清理 hw-helper/publish/..." -ForegroundColor White
        Remove-Item -Recurse -Force $helperPublish
    }

    # hw-helper resources
    $helperRes = Join-Path $root "src-tauri\resources\hw-helper"
    if (Test-Path $helperRes) {
        Write-Host "  ► 清理 src-tauri/resources/hw-helper/..." -ForegroundColor White
        Remove-Item -Recurse -Force $helperRes
    }

    Write-Host ""
    Write-Host "  ✓ 缓存清理完成" -ForegroundColor Green
}

# ─── 主循环 ────────────────────────────────────────────────────────────────────

while ($true) {
    Write-Header

    $choice = Read-Host "  请选择"
    $choice = $choice.Trim()

    switch ($choice) {
        "1" {
            try {
                Invoke-LtoBuild
            } catch {
                Write-Host ""
                Write-Host "  ✗ 编译失败: $_" -ForegroundColor Red
            }
            Write-Host ""
            Read-Host "  按回车返回菜单"
        }
        "2" {
            try {
                Invoke-NormalBuild
            } catch {
                Write-Host ""
                Write-Host "  ✗ 编译失败: $_" -ForegroundColor Red
            }
            Write-Host ""
            Read-Host "  按回车返回菜单"
        }
        "3" {
            try {
                Invoke-Clean
            } catch {
                Write-Host ""
                Write-Host "  ✗ 清理失败: $_" -ForegroundColor Red
            }
            Write-Host ""
            Read-Host "  按回车返回菜单"
        }
        "0" {
            Write-Host ""
            Write-Host "  再见！" -ForegroundColor DarkGray
            Write-Host ""
            exit 0
        }
        default {
            Write-Host "  无效选项，请重新输入" -ForegroundColor Red
            Start-Sleep -Seconds 1
        }
    }
}

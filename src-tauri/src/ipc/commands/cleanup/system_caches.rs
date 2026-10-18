use super::*;

pub(super) fn chromium_profile_cache_dirs(user_data: &Path) -> Vec<PathBuf> {
    let mut caches = Vec::new();
    let Some(root) = safe_cleanup_root(user_data) else {
        return caches;
    };
    // Opera/Electron 使用直接配置目录，Chromium 也有用户数据根级 shader 缓存。
    caches.extend(chromium_cache_dirs(root.clone()));
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if scan_should_stop() {
                break;
            }
            let profile = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&profile) else {
                continue;
            };
            if !metadata.is_dir() || metadata_is_reparse_point(&metadata) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "Default"
                || name == "System Profile"
                || name.starts_with("Profile ")
                || profile.join("Preferences").is_file()
            {
                caches.extend(chromium_cache_dirs(profile));
            }
        }
    }
    caches
        .into_iter()
        .filter_map(|path| safe_cleanup_root(&path))
        .collect()
}

pub(super) fn browser_cache_dirs(local: &Path, roaming: Option<&Path>) -> Vec<PathBuf> {
    let mut caches = Vec::new();
    for relative in [
        "Google/Chrome/User Data",
        "Google/Chrome Beta/User Data",
        "Google/Chrome SxS/User Data",
        "Microsoft/Edge/User Data",
        "Microsoft/Edge Beta/User Data",
        "Microsoft/Edge Dev/User Data",
        "BraveSoftware/Brave-Browser/User Data",
        "Vivaldi/User Data",
        "Chromium/User Data",
    ] {
        caches.extend(chromium_profile_cache_dirs(&local.join(relative)));
    }
    if let Some(roaming) = roaming {
        for relative in [
            "Opera Software/Opera Stable",
            "Opera Software/Opera GX Stable",
        ] {
            caches.extend(chromium_profile_cache_dirs(&roaming.join(relative)));
        }
    }
    if let Ok(profiles) = std::fs::read_dir(local.join("Mozilla/Firefox/Profiles")) {
        for profile in profiles.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(profile.path()) else {
                continue;
            };
            if metadata_is_reparse_point(&metadata) || !metadata.is_dir() {
                continue;
            }
            for name in ["cache2", "startupCache", "shader-cache"] {
                if let Some(cache) = safe_cleanup_root(&profile.path().join(name)) {
                    caches.push(cache);
                }
            }
        }
    }
    caches
}

pub(super) fn scan_browser_cache() -> Option<CleanupCategory> {
    scan_existing_dirs_category("browser-cache", "浏览器缓存",
        "Chrome / Edge / Brave / Vivaldi / Opera / Firefox 的磁盘、代码及 GPU 缓存（保留会话与站点数据）",
        browser_cache_dirs(&dirs::data_local_dir()?, dirs::data_dir().as_deref()))
}

pub(super) fn scan_webview_cache() -> Option<CleanupCategory> {
    let local = dirs::data_local_dir()?;
    let mut cache_dirs = Vec::new();

    for user_data in [
        local
            .join("Microsoft")
            .join("EdgeWebView")
            .join("User Data"),
        local.join("Microsoft").join("WebView2").join("EBWebView"),
    ] {
        cache_dirs.extend(chromium_profile_cache_dirs(&user_data));
    }

    scan_existing_dirs_category(
        "webview-cache",
        "WebView2 缓存",
        "Microsoft WebView2 Cache / Code Cache / GPUCache",
        cache_dirs,
    )
}

pub(super) fn scan_app_cache() -> Option<CleanupCategory> {
    let local = dirs::data_local_dir()?;
    let roaming = dirs::data_dir();
    let mut cache_dirs = Vec::new();

    for app in ["Discord", "discordcanary", "discordptb", "Slack"] {
        let root = local.join(app);
        for name in ["Cache", "Code Cache", "GPUCache", "DawnCache", "Crashpad"].iter() {
            let path = if *name == "Crashpad" {
                root.join(name).join("reports")
            } else {
                root.join(name)
            };
            push_cache_dir(&mut cache_dirs, path);
        }
    }

    for app in ["Code", "Cursor", "VSCodium"] {
        let root = local.join(app);
        for name in ["Cache", "Code Cache", "GPUCache", "Crashpad"].iter() {
            let path = if *name == "Crashpad" {
                root.join(name).join("reports")
            } else {
                root.join(name)
            };
            push_cache_dir(&mut cache_dirs, path);
        }
    }

    if let Some(roaming) = roaming {
        for app in [
            "Code",
            "Cursor",
            "VSCodium",
            "Slack",
            "Discord",
            "discordcanary",
            "discordptb",
        ] {
            let root = roaming.join(app);
            for name in [
                "Cache",
                "Code Cache",
                "GPUCache",
                "DawnCache",
                "DawnGraphiteCache",
                "DawnWebGPUCache",
            ]
            .iter()
            {
                push_cache_dir(&mut cache_dirs, root.join(name));
            }
        }
    }

    let teams = local.join("Microsoft").join("Teams");
    for name in [
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
    ]
    .iter()
    {
        push_cache_dir(&mut cache_dirs, teams.join(name));
    }

    scan_existing_dirs_category(
        "app-cache",
        "应用缓存",
        "Discord / Slack / Teams / VS Code 等应用的 Cache / GPUCache / Crashpad reports",
        cache_dirs,
    )
}

pub(super) fn scan_windows_error_reports() -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();

    if let Some(local) = dirs::data_local_dir() {
        let wer = local.join("Microsoft").join("Windows").join("WER");
        for name in ["ReportArchive", "ReportQueue", "Temp"].iter() {
            push_cache_dir(&mut cache_dirs, wer.join(name));
        }
        push_cache_dir(&mut cache_dirs, local.join("CrashDumps"));
    }

    let wer = PathBuf::from(r"C:\ProgramData\Microsoft\Windows\WER");
    for name in ["ReportArchive", "ReportQueue", "Temp"].iter() {
        push_cache_dir(&mut cache_dirs, wer.join(name));
    }

    scan_existing_dirs_category(
        "wer-cache",
        "错误报告缓存",
        "Windows 错误报告、崩溃转储与上报队列",
        cache_dirs,
    )
}

pub(super) fn scan_shader_cache() -> Option<CleanupCategory> {
    let local = dirs::data_local_dir()?;
    let mut cache_dirs = Vec::new();

    for path in [
        local.join("D3DSCache"),
        local.join("NVIDIA").join("DXCache"),
        local.join("NVIDIA").join("GLCache"),
        local.join("NVIDIA").join("ComputeCache"),
        local.join("AMD").join("DxCache"),
        local.join("AMD").join("GLCache"),
        local.join("AMD").join("VkCache"),
    ] {
        push_cache_dir(&mut cache_dirs, path);
    }

    scan_existing_dirs_category(
        "shader-cache",
        "着色器缓存",
        "DirectX / NVIDIA / AMD 可重新生成的 shader 缓存",
        cache_dirs,
    )
}

pub(super) fn scan_installer_cache() -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();

    if let Some(local) = dirs::data_local_dir() {
        push_cache_dir(&mut cache_dirs, local.join("SquirrelTemp"));
    }

    if let Some(home) = dirs::home_dir() {
        push_cache_dir(&mut cache_dirs, home.join("scoop").join("cache"));
    }

    push_cache_dir(
        &mut cache_dirs,
        PathBuf::from(r"C:\ProgramData\chocolatey\cache"),
    );

    scan_existing_dirs_category(
        "installer-cache",
        "安装器缓存",
        "Squirrel / Scoop / Chocolatey 下载缓存",
        cache_dirs,
    )
}

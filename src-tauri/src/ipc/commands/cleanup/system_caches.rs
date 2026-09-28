use super::*;

pub(super) fn scan_browser_cache() -> Option<CleanupCategory> {
    let local = dirs::data_local_dir()?;
    let mut browser_caches = Vec::new();

    for user_data in [
        local.join("Google").join("Chrome").join("User Data"),
        local.join("Microsoft").join("Edge").join("User Data"),
    ] {
        if let Ok(entries) = std::fs::read_dir(&user_data) {
            for entry in entries.flatten() {
                let profile = entry.path();
                if !profile.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "Default" || name.starts_with("Profile ") {
                    browser_caches.extend(chromium_cache_dirs(profile));
                }
            }
        }
    }

    browser_caches.push(local.join("Mozilla").join("Firefox").join("Profiles"));

    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();

    for cache_dir in &browser_caches {
        if !cache_dir.exists() {
            continue;
        }
        // For Firefox, scan cache2 subdirs
        if cache_dir.to_string_lossy().contains("Firefox") {
            if let Ok(entries) = std::fs::read_dir(cache_dir) {
                for entry in entries.flatten() {
                    let cache2 = entry.path().join("cache2");
                    if cache2.exists() {
                        let (s, c) = dir_size(&cache2);
                        if s > 0 {
                            size += s;
                            count += c;
                            paths.push(PathDetail {
                                path: cache2.to_string_lossy().to_string(),
                                size_bytes: s,
                                file_count: c,
                                ..path_detail_identity(&cache2)
                            });
                        }
                    }
                }
            }
        } else {
            let (s, c) = dir_size(cache_dir);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: cache_dir.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                    ..path_detail_identity(cache_dir)
                });
            }
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "browser-cache".to_string(),
        name: "浏览器缓存".to_string(),
        description: "Chrome / Edge / Firefox Cache / Code Cache / GPUCache".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("browser-cache").risk,
        default_selected: cleanup_policy("browser-cache").default_selected,
        min_age_days: cleanup_policy("browser-cache").min_age_days,
    })
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
        if let Ok(entries) = std::fs::read_dir(&user_data) {
            for entry in entries.flatten() {
                let profile = entry.path();
                if !profile.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "Default" || name.starts_with("Profile ") {
                    cache_dirs.extend(chromium_cache_dirs(profile));
                }
            }
        }
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
            for name in ["Cache", "Code Cache", "GPUCache"].iter() {
                push_cache_dir(&mut cache_dirs, root.join(name));
            }
        }
    }

    let teams = local.join("Microsoft").join("Teams");
    for name in ["Cache", "Code Cache", "GPUCache"].iter() {
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

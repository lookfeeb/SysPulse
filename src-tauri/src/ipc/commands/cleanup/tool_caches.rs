use super::*;

pub(super) fn scan_rust_targets(discovered_targets: &[PathBuf]) -> Option<CleanupCategory> {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();

    let mut targets = discovered_targets.to_vec();
    if let Some(cargo_cache) =
        dirs::home_dir().map(|home| home.join(".cargo").join("registry").join("cache"))
    {
        push_cache_dir(&mut targets, cargo_cache);
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let target_dir = PathBuf::from(target_dir);
        if is_rust_target_artifact_dir(&target_dir) {
            push_cache_dir(&mut targets, target_dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for config_path in [
            home.join(".cargo").join("config.toml"),
            home.join(".cargo").join("config"),
        ] {
            if let Some(target_dir) = cargo_target_dir_from_config(&config_path) {
                push_cache_dir(&mut targets, target_dir);
            }
        }
    }
    deduplicate_paths(&mut targets);

    for target_dir in &targets {
        let (s, c) = dir_size(target_dir);
        if s > 0 {
            size += s;
            count += c;
            paths.push(PathDetail {
                path: target_dir.to_string_lossy().to_string(),
                size_bytes: s,
                file_count: c,
                matched_rule: programming_matched_rule("rust-target", target_dir).to_string(),
                source: programming_path_source("rust-target", target_dir).to_string(),
                ..path_detail_identity(target_dir)
            });
        }
    }

    (size > 0).then(|| CleanupCategory {
        id: "rust-target".to_string(),
        name: "Rust 编译缓存".to_string(),
        description: "Cargo target 目录、显式 target-dir 和 registry 下载缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("rust-target").risk,
        default_selected: cleanup_policy("rust-target").default_selected,
        min_age_days: cleanup_policy("rust-target").min_age_days,
    })
}

pub(super) fn cargo_target_dir_from_config(config_path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_path).ok()?;
    let value = toml::from_str::<toml::Value>(&text).ok()?;
    let target_dir = value.get("build")?.get("target-dir")?.as_str()?;
    let path = PathBuf::from(target_dir);
    let config_dir = config_path.parent()?;
    let base_dir = if config_dir
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(".cargo"))
    {
        config_dir.parent().unwrap_or(config_dir)
    } else {
        config_dir
    };
    let resolved = if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    };
    let resolved = safe_cleanup_root(&resolved).unwrap_or(resolved);
    is_rust_target_artifact_dir(&resolved).then_some(resolved)
}

pub(super) fn cargo_config_target_from_project_root(project_root: &Path) -> Option<PathBuf> {
    let cargo_dir = project_root.join(".cargo");
    [cargo_dir.join("config.toml"), cargo_dir.join("config")]
        .into_iter()
        .find_map(|config| cargo_target_dir_from_config(&config))
}

pub(super) fn scan_node_cache() -> Option<CleanupCategory> {
    let mut dirs_to_scan: Vec<PathBuf> = Vec::new();

    // npm cache（纯下载缓存，等同于 npm cache clean）
    if let Some(local) = dirs::data_local_dir() {
        let npm_cache = local.join("npm-cache");
        if npm_cache.exists() {
            dirs_to_scan.push(npm_cache);
        }
        // Bun 下载缓存；pnpm store 是项目硬链接依赖，不作为可安全清理项。
        push_cache_dir(
            &mut dirs_to_scan,
            dirs::home_dir()
                .unwrap_or_default()
                .join(".bun")
                .join("install")
                .join("cache"),
        );
        push_cache_dir(
            &mut dirs_to_scan,
            local.join("bun").join("install").join("cache"),
        );
    }
    // yarn cache（纯下载缓存，等同于 yarn cache clean）
    if let Some(local) = dirs::data_local_dir() {
        let yarn = local.join("Yarn").join("Cache");
        if yarn.exists() {
            dirs_to_scan.push(yarn);
        }
    }
    // 注意：pnpm-store 是内容寻址存储，所有项目硬链接于此，不是缓存垃圾，不清理

    if dirs_to_scan.is_empty() {
        return None;
    }

    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();
    for d in &dirs_to_scan {
        let (s, c) = dir_size(d);
        if s > 0 {
            size += s;
            count += c;
            paths.push(PathDetail {
                path: d.to_string_lossy().to_string(),
                size_bytes: s,
                file_count: c,
                matched_rule: programming_matched_rule("node-cache", d).to_string(),
                source: programming_path_source("node-cache", d).to_string(),
                ..path_detail_identity(d)
            });
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "node-cache".to_string(),
        name: "Node.js 缓存".to_string(),
        description: "npm / Yarn / Bun 下载缓存（保留 pnpm 内容寻址存储）".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("node-cache").risk,
        default_selected: cleanup_policy("node-cache").default_selected,
        min_age_days: cleanup_policy("node-cache").min_age_days,
    })
}

pub(super) fn scan_python_cache(project_cache_dirs: &[PathBuf]) -> Option<CleanupCategory> {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();

    // pip cache
    if let Some(local) = dirs::data_local_dir() {
        let pip_cache = local.join("pip").join("cache");
        if pip_cache.exists() {
            let (s, c) = dir_size(&pip_cache);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: pip_cache.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                    matched_rule: programming_matched_rule("python-cache", &pip_cache).to_string(),
                    source: programming_path_source("python-cache", &pip_cache).to_string(),
                    ..path_detail_identity(&pip_cache)
                });
            }
        }
        for cache in [
            local.join("uv").join("cache"),
            local.join("pypoetry").join("Cache"),
            local.join("pdm").join("Cache"),
        ] {
            if cache.exists() {
                let (s, c) = dir_size(&cache);
                if s > 0 {
                    size += s;
                    count += c;
                    paths.push(PathDetail {
                        path: cache.to_string_lossy().to_string(),
                        size_bytes: s,
                        file_count: c,
                        matched_rule: programming_matched_rule("python-cache", &cache).to_string(),
                        source: programming_path_source("python-cache", &cache).to_string(),
                        ..path_detail_identity(&cache)
                    });
                }
            }
        }
    }

    // pipx cache
    if let Some(local) = dirs::data_local_dir() {
        let pipx = local.join("pipx").join("cache");
        if pipx.exists() {
            let (s, c) = dir_size(&pipx);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: pipx.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                    matched_rule: programming_matched_rule("python-cache", &pipx).to_string(),
                    source: programming_path_source("python-cache", &pipx).to_string(),
                    ..path_detail_identity(&pipx)
                });
            }
        }
    }

    // conda pkgs —— 只清压缩包缓存（*.tar.bz2 / *.conda），不能整目录删
    // pkgs/ 下的解压目录是 conda 环境通过硬链接引用的，删了会破坏所有环境
    if let Some(home) = dirs::home_dir() {
        for conda_dir in ["miniconda3", "anaconda3", "Miniconda3", "Anaconda3"] {
            let pkgs = home.join(conda_dir).join("pkgs");
            if pkgs.exists() {
                let (s, c) = conda_archive_size(&pkgs);
                if s > 0 {
                    size += s;
                    count += c;
                    paths.push(PathDetail {
                        path: pkgs.to_string_lossy().to_string(),
                        size_bytes: s,
                        file_count: c,
                        matched_rule: programming_matched_rule("python-cache", &pkgs).to_string(),
                        source: programming_path_source("python-cache", &pkgs).to_string(),
                        ..path_detail_identity(&pkgs)
                    });
                }
                break;
            }
        }
    }

    for cache in project_cache_dirs {
        let (s, c) = dir_size(cache);
        if s > 0 {
            size += s;
            count += c;
            paths.push(PathDetail {
                path: cache.to_string_lossy().to_string(),
                size_bytes: s,
                file_count: c,
                matched_rule: programming_matched_rule("python-cache", cache).to_string(),
                source: programming_path_source("python-cache", cache).to_string(),
                ..path_detail_identity(cache)
            });
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "python-cache".to_string(),
        name: "Python 缓存".to_string(),
        description: "pip / pipx / uv / Poetry / PDM / conda 下载缓存及已验证项目工具缓存"
            .to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("python-cache").risk,
        default_selected: cleanup_policy("python-cache").default_selected,
        min_age_days: cleanup_policy("python-cache").min_age_days,
    })
}

pub(super) fn scan_go_cache() -> Option<CleanupCategory> {
    // 只清 GOCACHE（编译缓存），不清 GOMODCACHE（模块源码缓存）。
    let go_cache = command_output_path("go", &["env", "GOCACHE"])
        .or_else(|| dirs::data_local_dir().map(|p| p.join("go-build")));

    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();
    if let Some(d) = &go_cache {
        if d.exists() {
            let (s, c) = dir_size(d);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: d.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                    matched_rule: programming_matched_rule("go-cache", d).to_string(),
                    source: programming_path_source("go-cache", d).to_string(),
                    ..path_detail_identity(d)
                });
            }
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "go-cache".to_string(),
        name: "Go 缓存".to_string(),
        description: "Go 编译缓存（go-build）".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("go-cache").risk,
        default_selected: cleanup_policy("go-cache").default_selected,
        min_age_days: cleanup_policy("go-cache").min_age_days,
    })
}

pub(super) fn command_output_path(command: &str, args: &[&str]) -> Option<PathBuf> {
    command_output_path_in(command, args, None)
}

pub(super) fn command_output_path_in(
    command: &str,
    args: &[&str],
    current_dir: Option<&Path>,
) -> Option<PathBuf> {
    let mut command = std::process::Command::new(command);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed)
            || started.elapsed() >= COMMAND_DISCOVERY_TIMEOUT
        {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait().ok()? {
            Some(_) => break,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().trim_matches('"');
    (!value.is_empty() && value != "off").then(|| PathBuf::from(value))
}

pub(super) fn scan_cpp_cache(discovered_build_dirs: &[PathBuf]) -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        for path in [
            local.join("ccache"),
            local.join("sccache"),
            local.join("vcpkg").join("archives"),
        ] {
            push_cache_dir(&mut cache_dirs, path);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for path in [home.join(".ccache"), home.join(".cache").join("sccache")] {
            push_cache_dir(&mut cache_dirs, path);
        }
    }
    for path in discovered_build_dirs {
        push_cache_dir(&mut cache_dirs, path.clone());
    }
    deduplicate_paths(&mut cache_dirs);
    scan_existing_dirs_category(
        "cpp-cache",
        "C/C++ 编译缓存",
        "已验证的 CMake/MSBuild 构建产物及 ccache/sccache/vcpkg 下载缓存（保留 Conan 包仓库）",
        cache_dirs,
    )
}

pub(super) fn scan_dotnet_cache() -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        push_cache_dir(&mut cache_dirs, local.join("NuGet").join("v3-cache"));
        push_cache_dir(&mut cache_dirs, local.join("NuGet").join("plugins-cache"));
        push_cache_dir(&mut cache_dirs, local.join("Temp").join("NuGetScratch"));
    }
    scan_existing_dirs_category(
        "dotnet-cache",
        ".NET/NuGet 缓存",
        "NuGet HTTP、插件与临时缓存（保留全局 packages 依赖目录）",
        cache_dirs,
    )
}

pub(super) fn scan_java_cache() -> Option<CleanupCategory> {
    let home = dirs::home_dir()?;
    let mut cache_dirs = Vec::new();
    for path in [
        home.join(".gradle").join("caches"),
        home.join(".gradle").join("wrapper").join("dists"),
        home.join(".m2").join("repository").join(".cache"),
    ] {
        push_cache_dir(&mut cache_dirs, path);
    }
    scan_existing_dirs_category(
        "java-cache",
        "Java 构建缓存",
        "Gradle 缓存与 wrapper 下载包（保留 Maven 本地依赖仓库）",
        cache_dirs,
    )
}

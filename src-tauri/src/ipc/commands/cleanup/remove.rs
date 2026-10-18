use super::*;

#[derive(Debug, Clone)]
pub(super) struct RunningProcessInfo {
    pub(super) name: String,
    pub(super) stem: String,
    pub(super) command_paths: Vec<String>,
    pub(super) cwd: Option<String>,
}

pub(super) fn running_process_snapshot() -> Vec<RunningProcessInfo> {
    let system = sysinfo::System::new_all();
    system
        .processes()
        .values()
        .map(|process| {
            let name = process.name().to_string_lossy().to_lowercase();
            RunningProcessInfo {
                stem: name.trim_end_matches(".exe").to_string(),
                name,
                command_paths: process
                    .cmd()
                    .iter()
                    .flat_map(|part| process_argument_path_keys(&part.to_string_lossy()))
                    .collect(),
                cwd: process
                    .cwd()
                    .map(|cwd| normalize_path_key(&cwd.to_string_lossy())),
            }
        })
        .collect()
}

pub(super) fn process_argument_path_keys(argument: &str) -> Vec<String> {
    let mut values = vec![normalize_path_key(argument)];
    if let Some((_, value)) = argument.split_once('=') {
        let normalized = normalize_path_key(value);
        if !normalized.is_empty() && !values.contains(&normalized) {
            values.push(normalized);
        }
    }
    values
}

pub(super) fn watched_processes(category_id: &str) -> Option<&'static [&'static str]> {
    let watched: &'static [&'static str] = match category_id {
        "browser-cache" => &["chrome", "msedge", "firefox", "brave", "opera"],
        "webview-cache" => &["msedgewebview2"],
        "app-cache" => &[
            "discord", "slack", "teams", "ms-teams", "code", "cursor", "vscodium",
        ],
        "notion-cache" => &["notion"],
        "rust-target" => &["cargo", "rustc"],
        "node-cache" => &["node", "npm", "npx", "yarn", "bun", "bunx"],
        "python-cache" => &[
            "python", "pythonw", "pip", "pip3", "pipx", "uv", "poetry", "pdm", "pytest", "mypy",
            "ruff", "conda",
        ],
        "go-cache" => &["go", "gofmt", "gopls"],
        "cpp-cache" => &[
            "cmake", "ctest", "ninja", "msbuild", "devenv", "cl", "clang", "clang-cl", "gcc",
            "g++", "ccache", "sccache", "meson", "xmake", "vcpkg",
        ],
        "dotnet-cache" => &["dotnet", "nuget", "msbuild", "devenv", "vstest.console"],
        "java-cache" => &[
            "gradle", "gradlew", "java", "javac", "mvn", "mvnw", "kotlin",
        ],
        _ => return None,
    };
    Some(watched)
}

pub(super) fn running_process_blockers_for_path(
    category_id: &str,
    path: &str,
    processes: &[RunningProcessInfo],
) -> Vec<String> {
    let Some(watched) = watched_processes(category_id) else {
        return Vec::new();
    };
    let cache_path = Path::new(path);
    // 只有已验证的项目产物关联项目根，全局下载缓存只匹配实际缓存路径。
    let project_root = project_cache_owner(category_id, cache_path);
    let normalized_path = normalize_path_key(
        &project_root
            .as_deref()
            .unwrap_or(cache_path)
            .to_string_lossy(),
    );
    let mut found = std::collections::BTreeSet::new();
    for process in processes {
        if !watched.contains(&process.stem.as_str()) {
            continue;
        }
        let uses_path = process
            .command_paths
            .iter()
            .any(|part| path_is_within_key(part, &normalized_path))
            || process
                .cwd
                .as_ref()
                .is_some_and(|cwd| path_is_within_key(cwd, &normalized_path));
        if uses_path {
            found.insert(process.name.clone());
        }
    }
    found.into_iter().collect()
}

pub(super) fn path_is_within_key(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub(super) fn project_cache_owner(category: &str, path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let is_project_cache = match category {
        "rust-target" => is_verified_rust_target(path, &name),
        "cpp-cache" => is_verified_cpp_build_dir(path, &name),
        "python-cache" => is_verified_python_project_cache(path, &name),
        "node-cache" | "dotnet-cache" | "java-cache" => {
            verified_extra_project_cache(path, &name) == Some(category)
        }
        _ => false,
    };
    if !is_project_cache {
        return None;
    }
    path.ancestors()
        .skip(1)
        .take(8)
        .find(|parent| directory_has_project_marker(parent))
        .map(Path::to_path_buf)
}

pub(super) fn record_cleanup_io_error(
    errors: &mut Vec<String>,
    path: &Path,
    error: &std::io::Error,
) {
    if errors.len() < MAX_SCAN_WARNINGS {
        let reason = match error.raw_os_error() {
            Some(32 | 33) => "文件正在使用，已跳过".to_string(),
            _ => error.to_string(),
        };
        errors.push(format!("{}: {}", path.display(), reason));
    } else if errors.len() == MAX_SCAN_WARNINGS {
        errors.push("还有其他文件未能处理，仅展示前 80 条文件异常；未处理文件均已保留".into());
    }
}

pub(super) fn paths_overlap_keys(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('\\'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub(super) fn normalize_path_key(value: &str) -> String {
    let normalized = value
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase();
    normalized
        .strip_prefix(r"\\?\unc\")
        .map(|rest| format!(r"\\{}", rest))
        .or_else(|| normalized.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or(normalized)
}

pub(super) fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = normalize_path_key(&left.to_string_lossy());
    let right = normalize_path_key(&right.to_string_lossy());
    paths_overlap_keys(&left, &right)
}

pub(super) fn remove_dir_contents_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    file_policy: CleanupFilePolicy,
    errors: &mut Vec<String>,
    diagnostics: &mut PathCleanupDiagnostics,
) -> std::io::Result<(u64, u64)> {
    walk_dir_with_progress(
        dir,
        progress,
        file_policy,
        CleanupWalk {
            depth: 0,
            max_depth: DIRECTORY_SCAN_MAX_DEPTH,
            remove_root: false,
        },
        errors,
        diagnostics,
    )
}
pub(super) fn remove_dir_tree_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    file_policy: CleanupFilePolicy,
    depth: u32,
    max_depth: u32,
    errors: &mut Vec<String>,
    diagnostics: &mut PathCleanupDiagnostics,
) -> std::io::Result<(u64, u64)> {
    walk_dir_with_progress(
        dir,
        progress,
        file_policy,
        CleanupWalk {
            depth,
            max_depth,
            remove_root: true,
        },
        errors,
        diagnostics,
    )
}

struct CleanupWalk {
    depth: u32,
    max_depth: u32,
    remove_root: bool,
}

fn walk_dir_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    file_policy: CleanupFilePolicy,
    walk: CleanupWalk,
    errors: &mut Vec<String>,
    diagnostics: &mut PathCleanupDiagnostics,
) -> std::io::Result<(u64, u64)> {
    let CleanupWalk {
        depth,
        max_depth,
        remove_root,
    } = walk;
    if depth > max_depth {
        errors.push(format!("{}: 超过最大清理深度 {}", dir.display(), max_depth));
        return Ok((0, 0));
    }
    let mut freed = 0u64;
    let mut count = 0u64;

    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("{}: 读取目录项失败: {}", dir.display(), error));
                continue;
            }
        };
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) => {
                errors.push(format!("{}: 读取元数据失败: {}", path.display(), error));
                continue;
            }
        };
        if metadata_is_reparse_point(&meta) {
            continue;
        }
        if meta.is_dir() {
            let decision = file_policy.directory_decision(&meta);
            if decision != CleanupPolicyDecision::Allow {
                diagnostics.record(&path, decision);
                continue;
            }
            let result = remove_dir_tree_with_progress(
                &path,
                progress,
                file_policy,
                depth + 1,
                max_depth,
                errors,
                diagnostics,
            );
            match result {
                Ok((s, c)) => {
                    freed += s;
                    count += c;
                }
                Err(error) => record_cleanup_io_error(errors, &path, &error),
            }
        } else {
            let decision = file_policy.file_decision(&meta);
            if decision != CleanupPolicyDecision::Allow {
                diagnostics.record(&path, decision);
                continue;
            }
            let size = file_details(&path, &meta)
                .map(|value| value.2)
                .unwrap_or_else(|| meta.len());
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    freed += size;
                    count += 1;
                    progress.add_result(1, size, 1);
                }
                Err(error) => {
                    record_cleanup_io_error(errors, &path, &error);
                    progress.fail(1);
                }
            }
        }
    }

    if remove_root {
        let _ = std::fs::remove_dir(dir);
    }
    Ok((freed, count))
}

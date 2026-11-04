use super::*;

#[derive(Debug, Clone)]
pub(super) struct RunningProcessInfo {
    pub(super) name: String,
    pub(super) stem: String,
    pub(super) command_paths: Vec<String>,
    pub(super) cwd: Option<String>,
}

pub(super) fn running_process_snapshot() -> Vec<RunningProcessInfo> {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_cwd(sysinfo::UpdateKind::OnlyIfNotSet),
    );
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
        "browser-cache" => &["chrome", "msedge", "firefox", "brave", "opera", "vivaldi"],
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
    let browser_owner = if category_id == "browser-cache" {
        [
            (r"\google\chrome\", "chrome"),
            (r"\microsoft\edge\", "msedge"),
            (r"\mozilla\firefox\", "firefox"),
            (r"\bravesoftware\", "brave"),
            (r"\opera software\", "opera"),
            (r"\vivaldi\", "vivaldi"),
        ]
        .into_iter()
        .find_map(|(marker, name)| normalized_path.contains(marker).then_some(name))
    } else {
        None
    };
    for process in processes {
        if !watched.contains(&process.stem.as_str()) {
            continue;
        }
        // A normal browser launch need not expose its profile/cache directory
        // in argv. Conservatively retain that browser's known cache roots.
        let uses_path = browser_owner == Some(process.stem.as_str())
            || process.command_paths.iter().any(|part| {
                path_is_within_key(part, &normalized_path)
                    || (category_id == "browser-cache"
                        && paths_overlap_keys(part, &normalized_path))
            })
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

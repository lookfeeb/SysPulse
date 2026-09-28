use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProjectScoutMode {
    Deep,
    FixedVolumeProbe,
}

#[derive(Clone)]
pub(super) struct ProjectSearchRoot {
    pub(super) path: PathBuf,
    pub(super) mode: ProjectScoutMode,
}

pub(super) fn push_project_search_root(
    roots: &mut Vec<ProjectSearchRoot>,
    path: PathBuf,
    mode: ProjectScoutMode,
) {
    if !path.is_dir() {
        return;
    }
    if let Some(existing) = roots
        .iter_mut()
        .find(|existing| same_path(&existing.path, &path))
    {
        if mode == ProjectScoutMode::Deep {
            existing.mode = ProjectScoutMode::Deep;
        }
        return;
    }
    roots.push(ProjectSearchRoot { path, mode });
}

pub(super) fn project_search_plan(
    custom_roots: &[PathBuf],
    fixed_roots: &[PathBuf],
) -> Vec<ProjectSearchRoot> {
    let mut roots = Vec::new();
    for root in custom_roots {
        push_project_search_root(&mut roots, root.clone(), ProjectScoutMode::Deep);
    }
    if let Some(home) = dirs::home_dir() {
        for path in [
            home.clone(),
            home.join("Desktop"),
            home.join("Documents"),
            home.join("Downloads"),
            home.join("OneDrive"),
            home.join("source").join("repos"),
            home.join("projects"),
            home.join("code"),
            home.join("src"),
            home.join("dev"),
        ] {
            push_project_search_root(&mut roots, path, ProjectScoutMode::Deep);
        }
    }
    for root in fixed_roots {
        // 常见开发容器直接深扫；卷根只做有预算的浅层侦察，命中项目标记后再提升。
        for path in dedicated_volume_roots(root) {
            push_project_search_root(&mut roots, path, ProjectScoutMode::Deep);
        }
        push_project_search_root(&mut roots, root.clone(), ProjectScoutMode::FixedVolumeProbe);
    }
    roots
}

#[cfg(test)]
pub(super) fn project_search_roots(custom_roots: &[PathBuf]) -> Vec<PathBuf> {
    let fixed_roots = fixed_disk_roots();
    project_search_plan(custom_roots, &fixed_roots)
        .into_iter()
        .map(|root| root.path)
        .collect()
}

#[derive(Default)]
pub(super) struct ProjectCacheDiscovery {
    pub(super) rust: Vec<PathBuf>,
    pub(super) cpp: Vec<PathBuf>,
    pub(super) python: Vec<PathBuf>,
}

#[derive(Clone)]
pub(super) struct ProjectScoutTask {
    pub(super) root: PathBuf,
    pub(super) dir: PathBuf,
    pub(super) depth: u32,
    pub(super) mode: ProjectScoutMode,
}

pub(super) struct ProjectScoutQueue {
    pub(super) tasks: Mutex<std::collections::VecDeque<ProjectScoutTask>>,
    pub(super) ready: Condvar,
    pub(super) pending: AtomicUsize,
}

impl ProjectScoutQueue {
    #[cfg(test)]
    pub(super) fn new(roots: &[PathBuf]) -> Self {
        let roots = roots
            .iter()
            .cloned()
            .map(|path| ProjectSearchRoot {
                path,
                mode: ProjectScoutMode::Deep,
            })
            .collect::<Vec<_>>();
        Self::from_plan(&roots)
    }

    pub(super) fn from_plan(roots: &[ProjectSearchRoot]) -> Self {
        let tasks = roots
            .iter()
            .map(|root| ProjectScoutTask {
                root: root.path.clone(),
                dir: root.path.clone(),
                depth: 0,
                mode: root.mode,
            })
            .collect::<std::collections::VecDeque<_>>();
        Self {
            pending: AtomicUsize::new(tasks.len()),
            tasks: Mutex::new(tasks),
            ready: Condvar::new(),
        }
    }

    pub(super) fn push(&self, task: ProjectScoutTask) {
        self.pending.fetch_add(1, Ordering::Relaxed);
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.push_back(task);
        self.ready.notify_one();
    }

    pub(super) fn pop(&self) -> Option<ProjectScoutTask> {
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(task) = tasks.pop_front() {
                return Some(task);
            }
            if self.pending.load(Ordering::Acquire) == 0 {
                return None;
            }
            tasks = self
                .ready
                .wait(tasks)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub(super) fn complete(&self) {
        let _tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.ready.notify_all();
        }
    }
}

pub(super) fn discover_project_caches(
    custom_roots: &[PathBuf],
    hotspot_roots: &[PathBuf],
    scout_workers: usize,
) -> ProjectCacheDiscovery {
    let mut discovery = ProjectCacheDiscovery::default();
    for path in hotspot_roots {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if is_verified_rust_target(path, &name) {
            discovery.rust.push(path.clone());
        } else if is_verified_cpp_build_dir(path, &name) {
            discovery.cpp.push(path.clone());
        } else if is_verified_python_project_cache(path, &name) {
            discovery.python.push(path.clone());
        }
    }

    let fixed_roots = fixed_disk_roots();
    let search_plan = project_search_plan(custom_roots, &fixed_roots);
    let roots = Arc::new(
        search_plan
            .iter()
            .map(|root| root.path.clone())
            .collect::<Vec<_>>(),
    );
    let queue = Arc::new(ProjectScoutQueue::from_plan(&search_plan));
    let volume_probe_tasks = Arc::new(AtomicUsize::new(
        search_plan
            .iter()
            .filter(|root| root.mode == ProjectScoutMode::FixedVolumeProbe)
            .count(),
    ));
    let volume_probe_limit_reported = Arc::new(AtomicBool::new(false));
    let found = Arc::new(AtomicUsize::new(
        discovery.rust.len() + discovery.cpp.len() + discovery.python.len(),
    ));
    let shared = Arc::new(Mutex::new(ProjectCacheDiscovery::default()));
    let workers = (0..scout_workers.max(1))
        .filter_map(|index| {
            let roots = roots.clone();
            let queue = queue.clone();
            let found = found.clone();
            let shared = shared.clone();
            let volume_probe_tasks = volume_probe_tasks.clone();
            let volume_probe_limit_reported = volume_probe_limit_reported.clone();
            std::thread::Builder::new()
                .name(format!("cleanup-scout-{index}"))
                .spawn(move || {
                    let mut local = ProjectCacheDiscovery::default();
                    while let Some(task) = queue.pop() {
                        scan_project_cache_task(
                            task,
                            roots.as_slice(),
                            queue.as_ref(),
                            found.as_ref(),
                            volume_probe_tasks.as_ref(),
                            volume_probe_limit_reported.as_ref(),
                            &mut local,
                        );
                        queue.complete();
                    }
                    if let Ok(mut result) = shared.lock() {
                        result.rust.append(&mut local.rust);
                        result.cpp.append(&mut local.cpp);
                        result.python.append(&mut local.python);
                    }
                })
                .ok()
        })
        .collect::<Vec<_>>();
    for worker in workers {
        let _ = worker.join();
    }
    if let Ok(mut concurrent) = shared.lock() {
        discovery.rust.append(&mut concurrent.rust);
        discovery.cpp.append(&mut concurrent.cpp);
        discovery.python.append(&mut concurrent.python);
    }
    deduplicate_paths(&mut discovery.rust);
    deduplicate_paths(&mut discovery.cpp);
    deduplicate_paths(&mut discovery.python);
    discovery
}

pub(super) fn normalize_custom_project_roots(roots: Vec<String>) -> Vec<PathBuf> {
    let mut paths = roots
        .into_iter()
        .take(16)
        .filter_map(|value| {
            let path = PathBuf::from(value.trim_matches('"'));
            safe_cleanup_root(&path).filter(|path| is_safe_project_root(path))
        })
        .collect::<Vec<_>>();
    deduplicate_paths(&mut paths);
    paths
}

pub(super) fn is_safe_project_root(path: &Path) -> bool {
    if is_volume_root(path) {
        return false;
    }
    let normalized = normalize_path_key(&path.to_string_lossy());
    let dangerous_roots = [
        std::env::var("WINDIR").ok().map(PathBuf::from),
        std::env::var("ProgramFiles").ok().map(PathBuf::from),
        std::env::var("ProgramFiles(x86)").ok().map(PathBuf::from),
        std::env::var("ProgramData").ok().map(PathBuf::from),
        dirs::data_local_dir(),
        dirs::data_dir(),
    ];
    !dangerous_roots.into_iter().flatten().any(|dangerous| {
        let dangerous = normalize_path_key(&dangerous.to_string_lossy());
        normalized == dangerous
            || dangerous
                .strip_prefix(&normalized)
                .is_some_and(|suffix| suffix.starts_with('\\'))
    })
}

pub(super) fn scan_project_cache_task(
    task: ProjectScoutTask,
    assigned_roots: &[PathBuf],
    queue: &ProjectScoutQueue,
    found: &AtomicUsize,
    volume_probe_tasks: &AtomicUsize,
    volume_probe_limit_reported: &AtomicBool,
    discovery: &mut ProjectCacheDiscovery,
) {
    let Some(diagnostics) = current_scan_diagnostics() else {
        return;
    };
    let dispatched = diagnostics.dispatch_scout();
    if dispatched == 1 || dispatched % 256 == 0 {
        diagnostics.progress("scout", "侦察蚁营正在深度检查项目目录", Some(&task.dir));
    }
    let mut scan_root = task.root.clone();
    let mut scan_depth = task.depth;
    let mut scan_mode = task.mode;
    if scan_mode == ProjectScoutMode::FixedVolumeProbe && directory_has_project_marker(&task.dir) {
        scan_root = task.dir.clone();
        scan_depth = 0;
        scan_mode = ProjectScoutMode::Deep;
    }
    if let Some(target) = cargo_config_target_from_project_root(&task.dir) {
        if !discovery
            .rust
            .iter()
            .any(|existing| same_path(existing, &target))
        {
            discovery.rust.push(target);
            found.fetch_add(1, Ordering::Relaxed);
        }
    }
    if !diagnostics.should_continue(&task.dir)
        || found.load(Ordering::Relaxed) >= MAX_PROJECT_CACHE_PATHS
    {
        if found.load(Ordering::Relaxed) >= MAX_PROJECT_CACHE_PATHS {
            diagnostics.warn(format!(
                "{}: 项目缓存候选超过 {} 项，已停止发现",
                task.root.display(),
                MAX_PROJECT_CACHE_PATHS
            ));
        }
        return;
    }
    let entries = match std::fs::read_dir(&task.dir) {
        Ok(entries) => entries,
        Err(error) => {
            diagnostics.warn(format!(
                "{}: 读取项目目录失败: {}",
                task.dir.display(),
                error
            ));
            return;
        }
    };
    for entry in entries {
        if !diagnostics.should_continue(&task.dir) {
            return;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.warn(format!(
                    "{}: 读取项目目录项失败: {}",
                    task.dir.display(),
                    error
                ));
                continue;
            }
        };
        let path = entry.path();
        if !diagnostics.try_visit(&path) {
            return;
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                diagnostics.warn(format!("{}: 读取元数据失败: {}", path.display(), error));
                continue;
            }
        };
        if !metadata.is_dir() {
            continue;
        }
        if metadata_is_reparse_point(&metadata) {
            diagnostics.skip_expected(1);
            continue;
        }
        if is_separately_assigned_root(&task.root, &path, assigned_roots) {
            diagnostics.skip_expected(1);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if is_verified_rust_target(&path, &name) {
            discovery.rust.push(path);
            found.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if is_verified_cpp_build_dir(&path, &name) {
            discovery.cpp.push(path);
            found.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if is_verified_python_project_cache(&path, &name) {
            discovery.python.push(path);
            found.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let next_mode = match scan_mode {
            ProjectScoutMode::Deep => {
                (scan_depth < project_search_depth(&scan_root)).then_some(ProjectScoutMode::Deep)
            }
            ProjectScoutMode::FixedVolumeProbe => {
                if is_project_container_name(&name) {
                    Some(ProjectScoutMode::Deep)
                } else if scan_depth < FIXED_VOLUME_PROBE_DEPTH {
                    Some(ProjectScoutMode::FixedVolumeProbe)
                } else if directory_has_project_marker(&path) {
                    // 到达浅侦察边界时仍检查下一层目录自身，避免漏掉名称不典型的项目根。
                    Some(ProjectScoutMode::Deep)
                } else {
                    None
                }
            }
        };
        if let Some(next_mode) = next_mode {
            if should_skip_project_path(&scan_root, &path, &name, scan_depth) {
                continue;
            }
            if next_mode == ProjectScoutMode::FixedVolumeProbe
                && volume_probe_tasks
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                        (value < MAX_FIXED_VOLUME_PROBE_TASKS).then_some(value + 1)
                    })
                    .is_err()
            {
                diagnostics.skip_expected(1);
                if !volume_probe_limit_reported.swap(true, Ordering::Relaxed) {
                    diagnostics.warn(format!(
                        "固定分区浅层侦察超过 {} 个目录预算，已保留已发现候选并停止扩展",
                        MAX_FIXED_VOLUME_PROBE_TASKS
                    ));
                }
                continue;
            }
            let promote = scan_mode == ProjectScoutMode::FixedVolumeProbe
                && next_mode == ProjectScoutMode::Deep;
            queue.push(ProjectScoutTask {
                root: if promote {
                    path.clone()
                } else {
                    scan_root.clone()
                },
                dir: path,
                depth: if promote { 0 } else { scan_depth + 1 },
                mode: next_mode,
            });
        }
    }
}

pub(super) fn is_project_container_name(name: &str) -> bool {
    const CONTAINER_NAMES: &[&str] = &[
        "code",
        "dev",
        "develop",
        "development",
        "git",
        "github",
        "gitlab",
        "project",
        "projects",
        "repo",
        "repos",
        "repository",
        "repositories",
        "source",
        "sources",
        "src",
        "workspace",
        "workspaces",
        "www",
    ];
    CONTAINER_NAMES.contains(&name)
}

pub(super) fn directory_has_project_marker(path: &Path) -> bool {
    const MARKERS: &[&str] = &[
        "cargo.toml",
        "package.json",
        "pnpm-workspace.yaml",
        "pyproject.toml",
        "setup.py",
        "go.mod",
        "cmakelists.txt",
        "meson.build",
        "xmake.lua",
        "build.gradle",
        "build.gradle.kts",
        "pom.xml",
    ];
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        MARKERS.contains(&name.as_str())
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "sln" | "vcxproj" | "csproj"
                    )
                })
    })
}

pub(super) fn is_separately_assigned_root(
    root: &Path,
    path: &Path,
    assigned_roots: &[PathBuf],
) -> bool {
    assigned_roots
        .iter()
        .any(|assigned| !same_path(assigned, root) && same_path(assigned, path))
}

pub(super) fn project_search_depth(root: &Path) -> u32 {
    if dirs::home_dir().is_some_and(|home| same_path(&home, root)) {
        return 5;
    }
    if is_volume_root(root) {
        return 4;
    }
    let is_broad_user_root = [
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
    ]
    .into_iter()
    .flatten()
    .any(|path| paths_overlap(&path, root) && paths_overlap(root, &path));
    if is_broad_user_root {
        7
    } else {
        9
    }
}

pub(super) fn should_skip_project_path(root: &Path, path: &Path, name: &str, depth: u32) -> bool {
    if name == ".git" || name == ".svn" || name == ".hg" {
        return true;
    }
    let always_skip = matches!(
        name,
        "appdata"
            | "node_modules"
            | "target"
            | "vendor"
            | "packages"
            | ".gradle"
            | ".m2"
            | ".nuget"
            | ".cargo"
            | ".rustup"
            | ".cache"
            | ".venv"
            | "venv"
            | "__pycache__"
            | "$recycle.bin"
            | "system volume information"
            | "windows"
            | "program files"
            | "program files (x86)"
            | "programdata"
            | "recovery"
            | "msocache"
    ) || (depth == 0 && name.starts_with('.'));
    if always_skip {
        return true;
    }
    if depth != 0 {
        return false;
    }
    if dirs::home_dir().is_some_and(|home| same_path(&home, root)) {
        return dedicated_user_roots()
            .iter()
            .any(|candidate| same_path(candidate, path))
            || matches!(
                name,
                "contacts"
                    | "favorites"
                    | "links"
                    | "music"
                    | "pictures"
                    | "saved games"
                    | "searches"
                    | "videos"
                    | "3d objects"
            );
    }
    if is_volume_root(root) {
        if name == "users" {
            return true;
        }
        return dedicated_volume_roots(root)
            .iter()
            .any(|candidate| same_path(candidate, path));
    }
    false
}

pub(super) fn dedicated_user_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        Some(home.join("OneDrive")),
        Some(home.join("source").join("repos")),
        Some(home.join("projects")),
        Some(home.join("code")),
        Some(home.join("src")),
        Some(home.join("dev")),
    ]
    .into_iter()
    .flatten()
    .filter(|path| path.exists())
    .collect()
}

pub(super) fn dedicated_volume_roots(root: &Path) -> Vec<PathBuf> {
    ["Code", "code", "Dev", "dev", "Projects", "projects", "src"]
        .into_iter()
        .map(|name| root.join(name))
        .filter(|path| path.exists())
        .collect()
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    if normalize_path_key(&left.to_string_lossy()) == normalize_path_key(&right.to_string_lossy()) {
        return true;
    }
    match (directory_identity(left), directory_identity(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

pub(super) fn is_volume_root(path: &Path) -> bool {
    path.parent().is_none() || path.parent().is_some_and(|parent| parent == path)
}

pub(super) fn is_verified_rust_target(path: &Path, name: &str) -> bool {
    if name != "target" || !is_rust_target_artifact_dir(path) {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    parent.join("Cargo.toml").is_file()
        || cargo_config_target_from_project_root(parent)
            .is_some_and(|target| same_path(&target, path))
}

pub(super) fn is_rust_target_artifact_dir(path: &Path) -> bool {
    path.join("CACHEDIR.TAG").is_file()
        || path.join(".rustc_info.json").is_file()
        || path.join("debug").is_dir()
        || path.join("release").is_dir()
}

pub(super) fn is_verified_cpp_build_dir(path: &Path, name: &str) -> bool {
    let cmake_metadata = path.join("CMakeCache.txt").is_file()
        || path.join("CMakeFiles").is_dir()
        || path.join("build.ninja").is_file();
    let meson_metadata = path.join("meson-private").is_dir() && path.join("build.ninja").is_file();
    let xmake_metadata = path.join(".gens").is_dir() || path.join(".objs").is_dir();
    let build_name = matches!(
        name,
        "build"
            | "build-debug"
            | "build-release"
            | "cmake-build-debug"
            | "cmake-build-release"
            | "cmake-build-relwithdebinfo"
    );
    if build_name && (cmake_metadata || meson_metadata || xmake_metadata) {
        return true;
    }
    if matches!(name, "debug" | "release" | "relwithdebinfo") {
        let parent = path.parent().unwrap_or(path);
        return parent.join("CMakeCache.txt").is_file() || parent.join("build.ninja").is_file();
    }
    if matches!(name, "obj" | "bin") {
        let Some(project_root) = path.parent() else {
            return false;
        };
        let has_msbuild_project = directory_has_extension(project_root, &["vcxproj", "sln"]);
        let has_generated_content = path.join("Debug").is_dir()
            || path.join("Release").is_dir()
            || directory_has_extension(path, &["obj", "pch", "pdb", "ilk", "tlog"]);
        return has_msbuild_project && has_generated_content;
    }
    false
}

pub(super) fn directory_has_extension(path: &Path, extensions: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions
                    .iter()
                    .any(|value| extension.eq_ignore_ascii_case(value))
            })
    })
}

pub(super) fn is_verified_python_project_cache(path: &Path, name: &str) -> bool {
    if !matches!(
        name,
        ".mypy_cache" | ".pytest_cache" | ".ruff_cache" | ".tox" | ".nox"
    ) {
        return false;
    }
    let Some(project_root) = path.parent() else {
        return false;
    };
    project_root.join("pyproject.toml").is_file()
        || project_root.join("setup.py").is_file()
        || project_root.join("setup.cfg").is_file()
        || project_root.join("tox.ini").is_file()
}

pub(super) fn deduplicate_directory_roots(paths: &mut Vec<PathBuf>) {
    let mut seen = std::collections::HashSet::new();
    let mut identities = std::collections::HashSet::new();
    paths.retain(|path| {
        let key = normalize_path_key(&path.to_string_lossy());
        if !seen.insert(key) {
            return false;
        }
        match directory_identity(path) {
            Some(identity) => identities.insert(identity),
            None => true,
        }
    });
}

pub(super) fn deduplicate_paths(paths: &mut Vec<PathBuf>) {
    deduplicate_directory_roots(paths);
}

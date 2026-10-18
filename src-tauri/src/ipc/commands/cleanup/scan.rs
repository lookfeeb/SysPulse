use super::*;

// ─── Scan logic ─────────────────────────────────────────────────────────────

pub(super) struct ScanDiagnostics {
    pub(super) scanned_paths: AtomicU64,
    pub(super) skipped_paths: AtomicU64,
    pub(super) ignored_paths: AtomicU64,
    pub(super) scout_tasks: AtomicU64,
    pub(super) engineer_tasks: AtomicU64,
    pub(super) warnings: Mutex<Vec<String>>,
    pub(super) started: Instant,
    pub(super) timeout_reported: AtomicBool,
    pub(super) budget_reported: AtomicBool,
    pub(super) truncated: AtomicBool,
}

impl Default for ScanDiagnostics {
    fn default() -> Self {
        Self {
            scanned_paths: AtomicU64::new(0),
            skipped_paths: AtomicU64::new(0),
            ignored_paths: AtomicU64::new(0),
            scout_tasks: AtomicU64::new(0),
            engineer_tasks: AtomicU64::new(0),
            warnings: Mutex::new(Vec::new()),
            started: Instant::now(),
            timeout_reported: AtomicBool::new(false),
            budget_reported: AtomicBool::new(false),
            truncated: AtomicBool::new(false),
        }
    }
}

impl ScanDiagnostics {
    pub(super) fn warn(&self, message: impl Into<String>) {
        self.skipped_paths.fetch_add(1, Ordering::Relaxed);
        self.append_warning(message);
    }

    pub(super) fn append_warning(&self, message: impl Into<String>) {
        if let Ok(mut warnings) = self.warnings.lock() {
            if warnings.len() < MAX_SCAN_WARNINGS {
                warnings.push(message.into());
            }
        }
    }

    pub(super) fn skip_expected(&self, count: u64) {
        self.ignored_paths.fetch_add(count, Ordering::Relaxed);
    }

    pub(super) fn dispatch_scout(&self) -> u64 {
        self.scout_tasks.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(super) fn dispatch_engineer(&self) -> u64 {
        self.engineer_tasks.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(super) fn try_visit(&self, current_path: &Path) -> bool {
        if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed) {
            return false;
        }
        if self.started.elapsed() >= MAX_SCAN_DURATION {
            self.truncated.store(true, Ordering::Relaxed);
            if !self.timeout_reported.swap(true, Ordering::Relaxed) {
                self.warn(format!(
                    "{}: 扫描超过 {} 秒预算，已停止并返回部分结果",
                    current_path.display(),
                    MAX_SCAN_DURATION.as_secs()
                ));
            }
            return false;
        }
        if self
            .scanned_paths
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                (value < MAX_SCAN_ENTRIES).then_some(value + 1)
            })
            .is_err()
        {
            self.truncated.store(true, Ordering::Relaxed);
            if !self.budget_reported.swap(true, Ordering::Relaxed) {
                self.warn(format!(
                    "{}: 扫描超过 {} 项预算，已停止并返回部分结果",
                    current_path.display(),
                    MAX_SCAN_ENTRIES
                ));
            }
            return false;
        }
        true
    }

    pub(super) fn should_continue(&self, current_path: &Path) -> bool {
        if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed) {
            return false;
        }
        if self.started.elapsed() >= MAX_SCAN_DURATION {
            self.truncated.store(true, Ordering::Relaxed);
            if !self.timeout_reported.swap(true, Ordering::Relaxed) {
                self.warn(format!(
                    "{}: 扫描超过 {} 秒预算，已停止并返回部分结果",
                    current_path.display(),
                    MAX_SCAN_DURATION.as_secs()
                ));
            }
            return false;
        }
        if self.scanned_paths.load(Ordering::Relaxed) >= MAX_SCAN_ENTRIES {
            self.truncated.store(true, Ordering::Relaxed);
            if !self.budget_reported.swap(true, Ordering::Relaxed) {
                self.warn(format!(
                    "{}: 扫描超过 {} 项预算，已停止并返回部分结果",
                    current_path.display(),
                    MAX_SCAN_ENTRIES
                ));
            }
            return false;
        }
        true
    }

    pub(super) fn snapshot(&self) -> (u64, u64, u64, Vec<String>) {
        (
            self.scanned_paths.load(Ordering::Relaxed),
            self.skipped_paths.load(Ordering::Relaxed),
            self.ignored_paths.load(Ordering::Relaxed),
            self.warnings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default(),
        )
    }
    pub(super) fn progress(&self, stage: &str, phase: &str, current_path: Option<&Path>) {
        if let Ok(active) = active_scan_app().lock() {
            if let Some(app) = active.as_ref() {
                let _ = app.emit(
                    CLEANUP_SCAN_PROGRESS_EVENT,
                    CleanupScanProgressEvent {
                        stage: stage.to_string(),
                        phase: phase.to_string(),
                        scanned_paths: self.scanned_paths.load(Ordering::Relaxed),
                        skipped_paths: self.skipped_paths.load(Ordering::Relaxed),
                        ignored_paths: self.ignored_paths.load(Ordering::Relaxed),
                        scout_tasks: self.scout_tasks.load(Ordering::Relaxed),
                        engineer_tasks: self.engineer_tasks.load(Ordering::Relaxed),
                        current_path: current_path.map(|path| path.to_string_lossy().to_string()),
                        done: false,
                        cancelled: SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed),
                    },
                );
            }
        }
    }
}

pub(super) fn do_scan(custom_project_roots: Vec<PathBuf>, colony: ColonyConfig) -> ScanResult {
    let diagnostics = Arc::new(ScanDiagnostics::default());
    let _diagnostics_guard = ActiveScanDiagnostics::install(diagnostics.clone());
    let directory_measures = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let _directory_measures_guard = ActiveScanDirectoryMeasures::install(directory_measures);
    let file_queue = Arc::new(FileScanQueue::start(colony.engineer_workers));
    let _file_queue_guard = ActiveScanFileQueue::install(file_queue);
    let mut categories = Vec::new();
    let previous_hotspots = load_hotspot_index();
    let active_hotspot_paths = hotspot_paths(&previous_hotspots);
    let hotspot_keys = active_hotspot_paths
        .iter()
        .map(|path| normalize_path_key(&path.to_string_lossy()))
        .collect::<std::collections::HashSet<_>>();

    diagnostics.progress("scout", "侦察蚁正在检查系统缓存", None);

    // 1. Windows Temp（TEMP/TMP 环境变量需经过路径合理性校验，防止误配置一锅端）
    if let Some(cat) = scan_dir_category(
        "win-temp",
        "Windows 临时文件",
        "系统和应用产生的临时文件",
        &[
            std::env::var("TEMP")
                .ok()
                .and_then(|s| validate_temp_dir(Path::new(&s))),
            std::env::var("TMP")
                .ok()
                .and_then(|s| validate_temp_dir(Path::new(&s))),
            Some(PathBuf::from(r"C:\Windows\Temp")),
        ],
    ) {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 2. (已移除 Windows Prefetch —— 系统自管理，清空后常用程序启动变慢)

    // 3. Windows Update cache
    if let Some(cat) = scan_dir_category(
        "win-update",
        "Windows 更新缓存",
        "已下载的 Windows 更新包",
        &[Some(PathBuf::from(
            r"C:\Windows\SoftwareDistribution\Download",
        ))],
    ) {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 5. Rust build cache (target dirs)
    if let Some(cat) = scan_rust_targets(&[]) {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 6. npm/pnpm/yarn cache
    if let Some(cat) = scan_node_cache() {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 6b. Go cache
    if let Some(cat) = scan_go_cache() {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 6c. Python cache (pip, __pycache__, .mypy_cache)
    if let Some(cat) = scan_python_cache(&[]) {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 6d. C/C++、.NET、Java 工具缓存与经过项目标记验证的构建产物
    if let Some(cat) = scan_cpp_cache(&[]) {
        categories.push(cat);
    }
    if let Some(cat) = scan_dotnet_cache() {
        categories.push(cat);
    }
    if let Some(cat) = scan_java_cache() {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 7. Browser caches
    diagnostics.progress("engineer", "工兵蚁正在统计浏览器与应用缓存", None);
    if let Some(cat) = scan_browser_cache() {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 7b. WebView2 runtime caches（只取 Cache / Code Cache / GPUCache 等可再生成目录）
    if let Some(cat) = scan_webview_cache() {
        categories.push(cat);
    }

    // 7c. 常见应用缓存（只取明确命名的 Cache / GPUCache / Crashpad reports）
    if let Some(cat) = scan_app_cache() {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 8. Thumbnail / Icon cache —— 只处理 thumbcache_*.db 和 iconcache_*.db 文件，
    //    Explorer 目录下还有 UsrClass.dat 等系统数据，绝不能整目录清空
    if let Some(local) = dirs::data_local_dir() {
        let explorer_cache = local.join("Microsoft").join("Windows").join("Explorer");
        if let Some(cat) = scan_thumbnail_cache(&explorer_cache) {
            categories.push(cat);
        }
    }

    // 9. (已移除 Chrome Update —— 那是 Chrome 更新程序的安装目录，不是缓存)

    // 10. Notion cache
    if let (Some(local), Some(roaming)) = (dirs::data_local_dir(), dirs::data_dir()) {
        if let Some(cat) = scan_dir_category(
            "notion-cache",
            "Notion 缓存",
            "Notion 应用本地缓存",
            &[
                Some(local.join("Notion").join("Cache")),
                Some(local.join("Notion").join("Code Cache")),
                Some(local.join("Notion").join("GPUCache")),
                Some(roaming.join("Notion").join("Cache")),
                Some(roaming.join("Notion").join("Code Cache")),
                Some(roaming.join("Notion").join("GPUCache")),
            ],
        ) {
            categories.push(cat);
        }
    }

    // 11. (已移除 OfficeFileCache —— 可能包含待同步/恢复内容，不作为垃圾缓存清理)

    // 12. Windows 错误报告与崩溃转储
    diagnostics.progress("engineer", "工兵蚁正在建立扫描结果", None);
    if let Some(cat) = scan_windows_error_reports() {
        categories.push(cat);
    }

    // 13. DirectX / GPU 着色器缓存
    if let Some(cat) = scan_shader_cache() {
        categories.push(cat);
    }

    // 14. 安装器残留缓存
    if let Some(cat) = scan_installer_cache() {
        categories.push(cat);
    }

    // 先完成确定位置的缓存，再进行固定盘/项目深扫，避免项目发现耗尽预算使后续类别完全漏扫。
    if !scan_should_stop() {
        diagnostics.progress("scout", "正在深度发现项目缓存，切换页面可继续扫描", None);
        let discovered = discover_project_caches(
            &custom_project_roots,
            &active_hotspot_paths,
            colony.scout_workers,
        );
        for (id, name, paths) in [
            ("rust-target", "Rust 编译缓存", discovered.rust),
            ("python-cache", "Python 缓存", discovered.python),
            ("cpp-cache", "C/C++ 编译缓存", discovered.cpp),
            ("node-cache", "Node.js 缓存", discovered.node),
            ("dotnet-cache", ".NET/NuGet 缓存", discovered.dotnet),
            ("java-cache", "Java 构建缓存", discovered.java),
        ] {
            if scan_should_stop() {
                break;
            }
            if let Some(category) =
                scan_existing_dirs_category(id, name, "已验证项目标记与构建缓存规则", paths)
            {
                merge_scan_category(&mut categories, category);
            }
        }
    }

    let mut result = finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    update_hotspot_index(
        previous_hotspots,
        &result.categories,
        chrono::Local::now().timestamp_millis(),
    );
    result.hotspot_count = load_hotspot_index().entries.len() as u64;
    result
}

pub(super) fn merge_scan_category(
    categories: &mut Vec<CleanupCategory>,
    mut extra: CleanupCategory,
) {
    if let Some(existing) = categories
        .iter_mut()
        .find(|category| category.id == extra.id)
    {
        existing.paths.append(&mut extra.paths);
    } else {
        categories.push(extra);
    }
    deduplicate_category_paths(categories);
}

pub(super) fn scan_should_stop() -> bool {
    SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed)
        || current_scan_diagnostics().is_some_and(|diagnostics| {
            diagnostics.started.elapsed() >= MAX_SCAN_DURATION
                || diagnostics.scanned_paths.load(Ordering::Relaxed) >= MAX_SCAN_ENTRIES
        })
}

pub(super) fn finalize_scan_result(
    mut categories: Vec<CleanupCategory>,
    diagnostics: &ScanDiagnostics,
    colony: ColonyConfig,
    hotspot_keys: &std::collections::HashSet<String>,
) -> ScanResult {
    deduplicate_category_paths(&mut categories);
    for category in &mut categories {
        for detail in &mut category.paths {
            if detail.matched_rule.is_empty() {
                detail.matched_rule =
                    programming_matched_rule(&category.id, Path::new(&detail.path)).to_string();
            }
            if detail.source.is_empty() {
                detail.source = if is_programming_category(&category.id) {
                    programming_path_source(&category.id, Path::new(&detail.path))
                } else {
                    path_source(&category.id, Path::new(&detail.path))
                }
                .to_string();
            }
            if hotspot_keys.contains(&normalize_path_key(&detail.path)) {
                detail.source = "hotspot-index".to_string();
            }
        }
    }
    let total_size_bytes = categories.iter().map(|c| c.size_bytes).sum();
    let total_file_count = categories.iter().map(|c| c.file_count).sum();
    let (scanned_paths, skipped_paths, ignored_paths, warnings) = diagnostics.snapshot();
    let cancelled = SCAN_CANCEL_REQUESTED.load(Ordering::SeqCst);

    ScanResult {
        scan_id: String::new(),
        categories,
        total_size_bytes,
        total_file_count,
        scanned_at_ms: 0,
        expires_at_ms: 0,
        duration_ms: 0,
        scanned_paths,
        skipped_paths,
        ignored_paths,
        hotspot_count: load_hotspot_index().entries.len() as u64,
        scout_workers: colony.scout_workers as u32,
        engineer_workers: colony.engineer_workers as u32,
        scout_tasks: diagnostics.scout_tasks.load(Ordering::Relaxed),
        engineer_tasks: diagnostics.engineer_tasks.load(Ordering::Relaxed),
        project_roots: Vec::new(),
        complete: !cancelled && !diagnostics.truncated.load(Ordering::Relaxed),
        cancelled,
        warnings,
    }
}

pub(super) fn deduplicate_category_paths(categories: &mut [CleanupCategory]) {
    let mut seen_paths = std::collections::HashSet::<String>::new();
    let mut seen_identities = std::collections::HashSet::<(u64, u64)>::new();
    for category in categories {
        category.paths.retain(|detail| {
            let path_key = normalize_path_key(&detail.path);
            if !seen_paths.insert(path_key) {
                return false;
            }
            match (detail.volume_serial, detail.file_id) {
                (Some(volume_serial), Some(file_id)) => {
                    seen_identities.insert((volume_serial, file_id))
                }
                _ => true,
            }
        });
        category.size_bytes = category.paths.iter().map(|path| path.size_bytes).sum();
        category.file_count = category.paths.iter().map(|path| path.file_count).sum();
    }
}

#[derive(Clone, Copy)]
pub(super) struct CleanupPolicy {
    pub(super) risk: CleanupRisk,
    pub(super) default_selected: bool,
    pub(super) min_age_days: Option<u32>,
}

pub(super) fn cleanup_policy(id: &str) -> CleanupPolicy {
    match id {
        "win-temp" => CleanupPolicy {
            risk: CleanupRisk::Safe,
            default_selected: true,
            min_age_days: Some(7),
        },
        "installer-cache" => CleanupPolicy {
            risk: CleanupRisk::Safe,
            default_selected: true,
            min_age_days: Some(14),
        },
        "browser-cache" | "app-cache" | "notion-cache" | "thumbnails" | "shader-cache"
        | "wer-cache" => CleanupPolicy {
            risk: CleanupRisk::Caution,
            default_selected: false,
            min_age_days: None,
        },
        _ => CleanupPolicy {
            risk: CleanupRisk::Advanced,
            default_selected: false,
            min_age_days: None,
        },
    }
}

pub(super) fn category_matched_rule(id: &str) -> &'static str {
    match id {
        "win-temp" => "临时目录 + 保留期",
        "win-update" => "Windows Update 下载目录",
        "rust-target" => "Cargo.toml + target 构建产物标记",
        "node-cache" => "包管理器缓存目录",
        "python-cache" => "Python 工具缓存或项目标记",
        "go-cache" => "go env GOCACHE",
        "cpp-cache" => "CMake/MSBuild 构建标记或编译缓存",
        "dotnet-cache" => "NuGet 可再生成缓存",
        "java-cache" => "Gradle/Maven 下载缓存",
        "browser-cache" => "浏览器配置目录 + 缓存子目录",
        "webview-cache" => "WebView2 缓存子目录",
        "app-cache" => "应用缓存白名单",
        "notion-cache" => "Notion 缓存白名单",
        "thumbnails" => "thumbcache/iconcache 文件名规则",
        "wer-cache" => "Windows 错误报告目录",
        "shader-cache" => "GPU 着色器缓存目录",
        "installer-cache" => "安装器临时缓存目录 + 保留期",
        _ => "已验证缓存目录",
    }
}

pub(super) fn is_programming_category(id: &str) -> bool {
    matches!(
        id,
        "rust-target"
            | "node-cache"
            | "python-cache"
            | "go-cache"
            | "cpp-cache"
            | "dotnet-cache"
            | "java-cache"
    )
}

pub(super) fn path_source(id: &str, path: &Path) -> &'static str {
    if matches!(id, "rust-target" | "cpp-cache" | "python-cache") && path.parent().is_some() {
        "project-discovery"
    } else if dirs::home_dir().is_some_and(|home| paths_overlap(&home, path)) {
        "user-directory"
    } else {
        "system-directory"
    }
}

pub(super) fn programming_matched_rule(id: &str, path: &Path) -> &'static str {
    let normalized = normalize_path_key(&path.to_string_lossy());
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if verified_extra_project_cache(path, &name) == Some(id) {
        return match id {
            "node-cache" => "package.json + 前端构建缓存目录",
            "dotnet-cache" => ".NET 项目文件 + NuGet 生成的 obj 标记",
            "java-cache" => "Gradle/Maven 项目文件 + 构建缓存标记",
            _ => category_matched_rule(id),
        };
    }
    match id {
        "rust-target" if normalized.ends_with(r"\.cargo\registry\cache") => {
            "Cargo registry 下载缓存"
        }
        "rust-target"
            if path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("target")) =>
        {
            "Cargo.toml + target 构建产物标记"
        }
        "rust-target" => "Cargo 显式 target-dir 构建产物标记",
        "node-cache" if normalized.contains(r"\npm-cache") => "npm 下载缓存目录",
        "node-cache" if normalized.contains(r"\yarn\cache") => "Yarn 下载缓存目录",
        "node-cache"
            if normalized.contains(r"\.bun\install\cache")
                || normalized.contains(r"\bun\install\cache") =>
        {
            "Bun 下载缓存目录"
        }
        "python-cache" if normalized.ends_with(r"\pip\cache") => "pip 下载缓存目录",
        "python-cache" if normalized.ends_with(r"\uv\cache") => "uv 下载缓存目录",
        "python-cache" if normalized.contains(r"\pypoetry\cache") => "Poetry 下载缓存目录",
        "python-cache" if normalized.contains(r"\pdm\cache") => "PDM 下载缓存目录",
        "python-cache" if normalized.ends_with(r"\pipx\cache") => "pipx 下载缓存目录",
        "python-cache" if normalized.ends_with(r"\pkgs") => "conda 压缩包缓存（仅归档文件）",
        "python-cache" => "pyproject/setup/tox 项目标记 + 工具缓存目录",
        "go-cache" => "go env GOCACHE 编译缓存",
        "cpp-cache" if normalized.ends_with(r"\ccache") => "ccache 编译缓存目录",
        "cpp-cache" if normalized.ends_with(r"\sccache") => "sccache 编译缓存目录",
        "cpp-cache" if normalized.ends_with(r"\vcpkg\archives") => "vcpkg 下载归档缓存",
        "cpp-cache" => "CMake/MSBuild/Meson/Xmake 构建产物标记",
        "dotnet-cache" if normalized.ends_with(r"\nuget\v3-cache") => "NuGet HTTP v3 缓存",
        "dotnet-cache" if normalized.ends_with(r"\nuget\plugins-cache") => "NuGet 插件缓存",
        "dotnet-cache" => "NuGet 临时缓存目录",
        "java-cache" if normalized.ends_with(r"\.gradle\wrapper\dists") => {
            "Gradle Wrapper 下载缓存"
        }
        "java-cache" if normalized.ends_with(r"\.gradle\caches") => "Gradle 可再生成缓存",
        "java-cache" => "Maven 本地仓库缓存子目录",
        _ => category_matched_rule(id),
    }
}

pub(super) fn programming_path_source(id: &str, path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if verified_extra_project_cache(path, &name) == Some(id) {
        return "project-discovery";
    }
    if matches!(id, "rust-target" | "python-cache" | "cpp-cache") {
        let normalized = normalize_path_key(&path.to_string_lossy());
        let global_markers = [
            r"\.cargo\registry\cache",
            r"\pip\cache",
            r"\uv\cache",
            r"\pypoetry\cache",
            r"\pdm\cache",
            r"\pipx\cache",
            r"\miniconda3\pkgs",
            r"\anaconda3\pkgs",
            r"\.ccache",
            r"\.cache\sccache",
            r"\appdata\local\ccache",
            r"\appdata\local\sccache",
            r"\appdata\local\vcpkg\archives",
        ];
        if global_markers
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            return "tool-global-cache";
        }
        return "project-discovery";
    }
    "tool-global-cache"
}

pub(super) fn scan_dir_category(
    id: &str,
    name: &str,
    description: &str,
    dirs: &[Option<PathBuf>],
) -> Option<CleanupCategory> {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let policy = cleanup_policy(id);
    let mut pending = Vec::new();

    for dir in dirs.iter().flatten() {
        if !dir.exists() {
            continue;
        }
        let canonical_dir = match safe_cleanup_root(dir) {
            Some(path) => path,
            None => continue,
        };
        let canonical = canonical_dir.to_string_lossy().to_lowercase();
        if !seen.insert(canonical) {
            continue;
        }
        let measurement = schedule_dir_size(&canonical_dir, policy.min_age_days);
        pending.push((canonical_dir, measurement));
    }

    for (canonical_dir, measurement) in pending {
        let (s, c) = measurement.wait();
        if s == 0 {
            continue;
        }
        size += s;
        count += c;
        paths.push(PathDetail {
            path: canonical_dir.to_string_lossy().to_string(),
            size_bytes: s,
            file_count: c,
            matched_rule: programming_matched_rule(id, &canonical_dir).to_string(),
            source: if is_programming_category(id) {
                programming_path_source(id, &canonical_dir)
            } else {
                path_source(id, &canonical_dir)
            }
            .to_string(),
            ..path_detail_identity(&canonical_dir)
        });
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: id.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: policy.risk,
        default_selected: policy.default_selected,
        min_age_days: policy.min_age_days,
    })
}

pub(super) fn scan_existing_dirs_category(
    id: &str,
    name: &str,
    description: &str,
    dirs: Vec<PathBuf>,
) -> Option<CleanupCategory> {
    let dirs: Vec<Option<PathBuf>> = dirs.into_iter().map(Some).collect();
    scan_dir_category(id, name, description, &dirs)
}

pub(super) fn push_cache_dir(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.exists() {
        paths.push(path);
    }
}

pub(super) fn queue_dir_size(
    path: &Path,
    min_age_days: Option<u32>,
    measurement: DirectoryMeasureCell,
) -> bool {
    let Some(queue) = current_scan_file_queue() else {
        return false;
    };
    queue
        .sender
        .send(FileScanJob {
            path: path.to_path_buf(),
            min_age_days,
            measurement,
        })
        .is_ok()
}

pub(super) fn fixed_disk_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        const DRIVE_FIXED: u32 = 3;

        let mask = unsafe { GetLogicalDrives() };
        for index in 0..26u32 {
            if mask & (1 << index) == 0 {
                continue;
            }
            let letter = (b'A' + index as u8) as char;
            let value = format!("{letter}:\\");
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } == DRIVE_FIXED {
                roots.push(PathBuf::from(value));
            }
        }
    }

    // sysinfo 在 Windows 上通过卷 GUID 枚举挂载点，可补齐没有盘符、仅挂载到目录的固定分区。
    for disk in sysinfo::Disks::new_with_refreshed_list().list() {
        let root = disk.mount_point().to_path_buf();
        if disk.is_removable() || !root.is_absolute() || !root.is_dir() {
            continue;
        }
        roots.push(root);
    }

    deduplicate_directory_roots(&mut roots);
    roots
}

pub(super) fn chromium_cache_dirs(profile_root: PathBuf) -> Vec<PathBuf> {
    [
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "DawnGraphiteCache",
        "DawnWebGPUCache",
        "ShaderCache",
        "GrShaderCache",
    ]
    .into_iter()
    .map(|name| profile_root.join(name))
    .collect()
}

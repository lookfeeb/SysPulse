use crate::error::IpcError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static CLEANUP_STATE: AtomicU8 = AtomicU8::new(CleanupTaskKind::Idle as u8);
static LAST_SCAN: OnceLock<Mutex<Option<ScanResult>>> = OnceLock::new();
const CLEANUP_PROGRESS_EVENT: &str = "cleanup:progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupTaskKind {
    Idle = 0,
    Scanning = 1,
    Cleaning = 2,
    LargeScanning = 3,
}

struct CleanupTaskGuard {
    kind: CleanupTaskKind,
}

impl CleanupTaskGuard {
    fn acquire(kind: CleanupTaskKind) -> Result<Self, IpcError> {
        CLEANUP_STATE
            .compare_exchange(
                CleanupTaskKind::Idle as u8,
                kind as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map(|_| Self { kind })
            .map_err(|current| {
                crate::error::AppError::Invalid(format!(
                    "cleanup task already in progress: {}",
                    cleanup_state_name(current)
                ))
                .into()
            })
    }
}

impl Drop for CleanupTaskGuard {
    fn drop(&mut self) {
        let _ = self.kind;
        CLEANUP_STATE.store(CleanupTaskKind::Idle as u8, Ordering::SeqCst);
    }
}

fn cleanup_state_name(value: u8) -> &'static str {
    match value {
        value if value == CleanupTaskKind::Scanning as u8 => "scanning",
        value if value == CleanupTaskKind::Cleaning as u8 => "cleaning",
        value if value == CleanupTaskKind::LargeScanning as u8 => "large-scanning",
        _ => "idle",
    }
}

fn scan_cache() -> &'static Mutex<Option<ScanResult>> {
    LAST_SCAN.get_or_init(|| Mutex::new(None))
}

fn cached_scan() -> Option<ScanResult> {
    scan_cache().lock().ok().and_then(|guard| guard.clone())
}

fn store_scan(scan: &ScanResult) {
    if let Ok(mut guard) = scan_cache().lock() {
        *guard = Some(scan.clone());
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PathDetail {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCategory {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub paths: Vec<PathDetail>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub categories: Vec<CleanupCategory>,
    pub total_size_bytes: u64,
    pub total_file_count: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileScanResult {
    pub files: Vec<LargeFile>,
    pub total_scanned: u64,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub freed_bytes: u64,
    pub deleted_files: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupProgressEvent {
    pub percent: u8,
    pub processed_items: u64,
    pub total_items: u64,
    pub current_category: String,
    pub current_path: Option<String>,
    pub freed_bytes: u64,
    pub deleted_files: u64,
    pub done: bool,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanArgs {
    pub category_ids: Vec<String>,
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileScanArgs {
    pub root: String,
    pub min_size_mb: u64,
    pub limit: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn scan_cleanup() -> Result<ScanResult, IpcError> {
    let _guard = CleanupTaskGuard::acquire(CleanupTaskKind::Scanning)?;
    let result = tokio::task::spawn_blocking(do_scan)
        .await
        .map_err(|e| crate::error::AppError::Other(format!("cleanup scan join: {e}")))?;
    store_scan(&result);
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn clean_categories(app: AppHandle, args: CleanArgs) -> Result<CleanResult, IpcError> {
    let _guard = CleanupTaskGuard::acquire(CleanupTaskKind::Cleaning)?;
    let ids = args.category_ids;
    let excluded = args.excluded_paths;
    let scan = cached_scan();
    let result = tokio::task::spawn_blocking(move || do_clean(&app, scan, &ids, &excluded))
        .await
        .map_err(|e| crate::error::AppError::Other(format!("cleanup clean join: {e}")))?;
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn scan_large_files(args: LargeFileScanArgs) -> Result<LargeFileScanResult, IpcError> {
    let _guard = CleanupTaskGuard::acquire(CleanupTaskKind::LargeScanning)?;
    let result = tokio::task::spawn_blocking(move || do_scan_large_files(&args))
        .await
        .map_err(|e| crate::error::AppError::Other(format!("large-file scan join: {e}")))?;
    Ok(result)
}

// ─── Scan logic ─────────────────────────────────────────────────────────────

fn do_scan() -> ScanResult {
    let mut categories = Vec::new();

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

    // 4. Recycle Bin
    if let Some(cat) = scan_dir_category(
        "recycle-bin",
        "回收站",
        "已删除但未清空的文件",
        &[Some(PathBuf::from(r"C:\$Recycle.Bin"))],
    ) {
        categories.push(cat);
    }

    // 5. Rust build cache (target dirs)
    categories.push(scan_rust_targets());

    // 6. npm/pnpm/yarn cache
    if let Some(cat) = scan_node_cache() {
        categories.push(cat);
    }

    // 6b. Go cache
    if let Some(cat) = scan_go_cache() {
        categories.push(cat);
    }

    // 6c. Python cache (pip, __pycache__, .mypy_cache)
    if let Some(cat) = scan_python_cache() {
        categories.push(cat);
    }

    // 7. Browser caches
    if let Some(cat) = scan_browser_cache() {
        categories.push(cat);
    }

    // 7b. WebView2 runtime caches（只取 Cache / Code Cache / GPUCache 等可再生成目录）
    if let Some(cat) = scan_webview_cache() {
        categories.push(cat);
    }

    // 7c. 常见应用缓存（只取明确命名的 Cache / GPUCache / Crashpad reports）
    if let Some(cat) = scan_app_cache() {
        categories.push(cat);
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
    if let Some(local) = dirs::data_local_dir() {
        if let Some(cat) = scan_dir_category(
            "notion-cache",
            "Notion 缓存",
            "Notion 应用本地缓存",
            &[
                Some(local.join("Notion").join("Cache")),
                Some(local.join("Notion").join("Code Cache")),
                Some(local.join("Notion").join("GPUCache")),
            ],
        ) {
            categories.push(cat);
        }
    }

    // 11. (已移除 OfficeFileCache —— 可能包含待同步/恢复内容，不作为垃圾缓存清理)

    // 12. Windows 错误报告与崩溃转储
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

    let total_size_bytes = categories.iter().map(|c| c.size_bytes).sum();
    let total_file_count = categories.iter().map(|c| c.file_count).sum();

    ScanResult {
        categories,
        total_size_bytes,
        total_file_count,
    }
}

fn scan_dir_category(
    id: &str,
    name: &str,
    description: &str,
    dirs: &[Option<PathBuf>],
) -> Option<CleanupCategory> {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dir in dirs.iter().flatten() {
        if !dir.exists() {
            continue;
        }
        let canonical = dir.to_string_lossy().to_string();
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let (s, c) = dir_size(dir);
        if s == 0 {
            continue;
        }
        size += s;
        count += c;
        paths.push(PathDetail {
            path: canonical,
            size_bytes: s,
            file_count: c,
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
    })
}

fn scan_existing_dirs_category(
    id: &str,
    name: &str,
    description: &str,
    dirs: Vec<PathBuf>,
) -> Option<CleanupCategory> {
    let dirs: Vec<Option<PathBuf>> = dirs.into_iter().map(Some).collect();
    scan_dir_category(id, name, description, &dirs)
}

fn push_cache_dir(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.exists() {
        paths.push(path);
    }
}

fn chromium_cache_dirs(profile_root: PathBuf) -> Vec<PathBuf> {
    [
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "ShaderCache",
        "GrShaderCache",
    ]
    .into_iter()
    .map(|name| profile_root.join(name))
    .collect()
}

fn scan_rust_targets() -> CleanupCategory {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();

    let search_roots: Vec<PathBuf> = [
        dirs::home_dir().map(|h| h.join("projects")),
        dirs::home_dir().map(|h| h.join("code")),
        dirs::home_dir().map(|h| h.join("src")),
        dirs::home_dir().map(|h| h.join("dev")),
        dirs::home_dir().map(|h| h.join("Desktop")),
        dirs::home_dir().map(|h| h.join("Documents")),
        dirs::home_dir().map(|h| h.join(".cargo").join("registry").join("cache")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.exists())
    .collect();

    for root in &search_roots {
        if root.ends_with("cache") && root.to_string_lossy().contains(".cargo") {
            let (s, c) = dir_size(root);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: root.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                });
            }
            continue;
        }
        find_rust_targets(root, 0, 4, &mut |target_dir| {
            let (s, c) = dir_size(target_dir);
            if s > 0 {
                size += s;
                count += c;
                paths.push(PathDetail {
                    path: target_dir.to_string_lossy().to_string(),
                    size_bytes: s,
                    file_count: c,
                });
            }
        });
    }

    CleanupCategory {
        id: "rust-target".to_string(),
        name: "Rust 编译缓存".to_string(),
        description: "Cargo target 目录和 registry 缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
    }
}

fn find_rust_targets(dir: &Path, depth: u32, max_depth: u32, cb: &mut dyn FnMut(&Path)) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "target" {
            // Verify it's a Rust target dir
            if path.join("CACHEDIR.TAG").exists()
                || path.join(".rustc_info.json").exists()
                || path.join("debug").exists()
                || path.join("release").exists()
            {
                cb(&path);
                continue;
            }
        }
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        find_rust_targets(&path, depth + 1, max_depth, cb);
    }
}

fn scan_node_cache() -> Option<CleanupCategory> {
    let mut dirs_to_scan: Vec<PathBuf> = Vec::new();

    // npm cache（纯下载缓存，等同于 npm cache clean）
    if let Some(local) = dirs::data_local_dir() {
        let npm_cache = local.join("npm-cache");
        if npm_cache.exists() {
            dirs_to_scan.push(npm_cache);
        }
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
            });
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "node-cache".to_string(),
        name: "Node.js 缓存".to_string(),
        description: "npm / pnpm / yarn 包缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
    })
}

fn scan_python_cache() -> Option<CleanupCategory> {
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
                });
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
                    });
                }
                break;
            }
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "python-cache".to_string(),
        name: "Python 缓存".to_string(),
        description: "pip / conda 包缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
    })
}

fn scan_go_cache() -> Option<CleanupCategory> {
    // 只清 go-build（编译缓存），不清 mod/cache（模块源码缓存，清掉会破坏 go.sum 校验）
    let go_cache = dirs::data_local_dir().map(|p| p.join("go-build"));

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
    })
}

fn scan_browser_cache() -> Option<CleanupCategory> {
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
    })
}

fn scan_webview_cache() -> Option<CleanupCategory> {
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

fn scan_app_cache() -> Option<CleanupCategory> {
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
        for name in ["Cache", "CachedData", "Code Cache", "GPUCache", "Crashpad"].iter() {
            let path = if *name == "Crashpad" {
                root.join(name).join("reports")
            } else {
                root.join(name)
            };
            push_cache_dir(&mut cache_dirs, path);
        }
    }

    if let Some(roaming) = roaming {
        for app in ["Code", "Cursor", "VSCodium", "Slack"] {
            let root = roaming.join(app);
            for name in ["Cache", "CachedData", "Code Cache", "GPUCache"].iter() {
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

fn scan_windows_error_reports() -> Option<CleanupCategory> {
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

fn scan_shader_cache() -> Option<CleanupCategory> {
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

fn scan_installer_cache() -> Option<CleanupCategory> {
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

// ─── Clean logic ────────────────────────────────────────────────────────────

struct CleanProgress<'a> {
    app: &'a AppHandle,
    total_items: u64,
    processed_items: u64,
    current_category: String,
    current_path: Option<String>,
    freed_bytes: u64,
    deleted_files: u64,
    last_emit: Instant,
}

impl<'a> CleanProgress<'a> {
    fn new(app: &'a AppHandle, total_items: u64) -> Self {
        Self {
            app,
            total_items,
            processed_items: 0,
            current_category: String::new(),
            current_path: None,
            freed_bytes: 0,
            deleted_files: 0,
            last_emit: Instant::now() - Duration::from_millis(250),
        }
    }

    fn set_current(&mut self, category: &str, path: Option<&str>) {
        self.current_category = category.to_string();
        self.current_path = path.map(|p| p.to_string());
        self.emit(false, false);
    }

    fn add_result(&mut self, processed: u64, freed: u64, deleted: u64) {
        self.processed_items = self.processed_items.saturating_add(processed);
        self.freed_bytes = self.freed_bytes.saturating_add(freed);
        self.deleted_files = self.deleted_files.saturating_add(deleted);
        self.emit(false, false);
    }

    fn skip(&mut self, processed: u64) {
        self.processed_items = self.processed_items.saturating_add(processed);
        self.emit(false, false);
    }

    fn finish(&mut self) {
        self.processed_items = self.total_items;
        self.emit(true, true);
    }

    fn emit(&mut self, force: bool, done: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_millis(120) {
            return;
        }
        let percent = if self.total_items == 0 {
            if done {
                100
            } else {
                0
            }
        } else {
            let value = self.processed_items.saturating_mul(100) / self.total_items;
            value.min(if done { 100 } else { 99 }) as u8
        };
        let _ = self.app.emit(
            CLEANUP_PROGRESS_EVENT,
            &CleanupProgressEvent {
                percent,
                processed_items: self.processed_items,
                total_items: self.total_items,
                current_category: self.current_category.clone(),
                current_path: self.current_path.clone(),
                freed_bytes: self.freed_bytes,
                deleted_files: self.deleted_files,
                done,
            },
        );
        self.last_emit = Instant::now();
    }
}

fn do_clean(
    app: &AppHandle,
    scan: Option<ScanResult>,
    category_ids: &[String],
    excluded_paths: &[String],
) -> CleanResult {
    let scan = scan.unwrap_or_else(do_scan);
    let mut errors = Vec::new();
    let selected_ids: std::collections::HashSet<&str> =
        category_ids.iter().map(|id| id.as_str()).collect();
    let excluded_set: std::collections::HashSet<&str> =
        excluded_paths.iter().map(|s| s.as_str()).collect();
    let selected_categories: Vec<&CleanupCategory> = scan
        .categories
        .iter()
        .filter(|cat| selected_ids.contains(cat.id.as_str()))
        .collect();
    for unknown in selected_ids
        .iter()
        .filter(|id| !scan.categories.iter().any(|cat| cat.id.as_str() == **id))
    {
        errors.push(format!("未知清理类别: {unknown}"));
    }
    let total_items = selected_categories
        .iter()
        .flat_map(|cat| cat.paths.iter())
        .filter(|detail| !excluded_set.contains(detail.path.as_str()))
        .map(|detail| detail.file_count.max(1))
        .sum();
    let mut progress = CleanProgress::new(app, total_items);
    progress.emit(true, false);

    for cat in selected_categories {
        match cat.id.as_str() {
            // 回收站：必须走 SHEmptyRecycleBinW，直接遍历删除会破坏 $Recycle.Bin 结构
            "recycle-bin" => match empty_recycle_bin() {
                Ok(()) => {
                    progress.set_current(&cat.name, Some("回收站"));
                    progress.add_result(cat.file_count.max(1), cat.size_bytes, cat.file_count);
                }
                Err(e) => {
                    errors.push(format!("回收站: {}", e));
                    progress.skip(cat.file_count.max(1));
                }
            },

            // 缩略图：只删 thumbcache_*.db / iconcache_*.db，不能整目录清空
            "thumbnails" => {
                for detail in &cat.paths {
                    if excluded_set.contains(detail.path.as_str()) {
                        continue;
                    }
                    let path = Path::new(&detail.path);
                    if !path.exists() {
                        progress.skip(detail.file_count.max(1));
                        continue;
                    }
                    progress.set_current(&cat.name, Some(&detail.path));
                    match remove_thumbnail_files_with_progress(path, &mut progress) {
                        Ok((_s, _c)) => {}
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            progress.skip(detail.file_count.max(1));
                        }
                    }
                }
            }

            // Python 缓存：conda pkgs 只删压缩包，其他正常清空
            "python-cache" => {
                for detail in &cat.paths {
                    if excluded_set.contains(detail.path.as_str()) {
                        continue;
                    }
                    let path = Path::new(&detail.path);
                    if !path.exists() {
                        progress.skip(detail.file_count.max(1));
                        continue;
                    }
                    progress.set_current(&cat.name, Some(&detail.path));
                    let is_conda_pkgs = path.ends_with("pkgs")
                        && path.to_string_lossy().to_lowercase().contains("conda");
                    let result = if is_conda_pkgs {
                        remove_conda_archives_with_progress(path, &mut progress)
                    } else {
                        remove_dir_contents_with_progress(path, &mut progress)
                    };
                    match result {
                        Ok((_s, _c)) => {}
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            progress.skip(detail.file_count.max(1));
                        }
                    }
                }
            }

            // 其他类别：清空目录内容
            _ => {
                for detail in &cat.paths {
                    if excluded_set.contains(detail.path.as_str()) {
                        continue;
                    }
                    let path = Path::new(&detail.path);
                    if !path.exists() {
                        progress.skip(detail.file_count.max(1));
                        continue;
                    }
                    progress.set_current(&cat.name, Some(&detail.path));
                    match remove_dir_contents_with_progress(path, &mut progress) {
                        Ok((_s, _c)) => {}
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            progress.skip(detail.file_count.max(1));
                        }
                    }
                }
            }
        }
    }

    progress.finish();
    CleanResult {
        freed_bytes: progress.freed_bytes,
        deleted_files: progress.deleted_files,
        errors,
    }
}

fn remove_dir_contents_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
) -> std::io::Result<(u64, u64)> {
    let mut freed = 0u64;
    let mut count = 0u64;

    let entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    for entry in entries {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            let (s, c) = remove_dir_tree_with_progress(&path, progress)?;
            freed += s;
            count += c;
        } else {
            let size = meta.len();
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
                count += 1;
                progress.add_result(1, size, 1);
            } else {
                progress.skip(1);
            }
        }
    }

    Ok((freed, count))
}

fn remove_dir_tree_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
) -> std::io::Result<(u64, u64)> {
    let mut freed = 0u64;
    let mut count = 0u64;

    let entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            let (s, c) = remove_dir_tree_with_progress(&path, progress)?;
            freed += s;
            count += c;
        } else {
            let size = meta.len();
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
                count += 1;
                progress.add_result(1, size, 1);
            } else {
                progress.skip(1);
            }
        }
    }

    let _ = std::fs::remove_dir(dir);
    Ok((freed, count))
}

// ─── Large file scan ────────────────────────────────────────────────────────

fn do_scan_large_files(args: &LargeFileScanArgs) -> LargeFileScanResult {
    let min_bytes = args.min_size_mb * 1024 * 1024;
    let limit = args.limit.min(500) as usize;
    let mut files: Vec<LargeFile> = Vec::new();
    let mut total_scanned = 0u64;

    let root = Path::new(&args.root);
    if !root.exists() {
        return LargeFileScanResult {
            files,
            total_scanned: 0,
        };
    }

    scan_large_recursive(
        root,
        min_bytes,
        limit,
        &mut files,
        &mut total_scanned,
        0,
        20,
    );
    files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    files.truncate(limit);

    LargeFileScanResult {
        files,
        total_scanned,
    }
}

fn scan_large_recursive(
    dir: &Path,
    min_bytes: u64,
    limit: usize,
    results: &mut Vec<LargeFile>,
    total_scanned: &mut u64,
    depth: u32,
    max_depth: u32,
) {
    if depth > max_depth {
        return;
    }
    if results.len() >= limit.saturating_mul(4).max(limit) {
        compact_large_file_results(results, limit);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if meta.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Skip system dirs that are slow/inaccessible
            if name_str.starts_with('$') || name_str == "System Volume Information" {
                continue;
            }
            scan_large_recursive(
                &path,
                min_bytes,
                limit,
                results,
                total_scanned,
                depth + 1,
                max_depth,
            );
        } else {
            *total_scanned += 1;
            let size = meta.len();
            if size >= min_bytes {
                results.push(LargeFile {
                    path: path.to_string_lossy().to_string(),
                    size_bytes: size,
                });
                if results.len() >= limit.saturating_mul(4).max(limit) {
                    compact_large_file_results(results, limit);
                }
            }
        }
    }
}

fn compact_large_file_results(results: &mut Vec<LargeFile>, limit: usize) {
    if limit == 0 {
        results.clear();
        return;
    }
    results.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    results.truncate(limit);
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn dir_size(path: &Path) -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;
    dir_size_recursive(path, &mut size, &mut count, 0, 15);
    (size, count)
}

fn dir_size_recursive(path: &Path, size: &mut u64, count: &mut u64, depth: u32, max_depth: u32) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            dir_size_recursive(&entry.path(), size, count, depth + 1, max_depth);
        } else {
            *size += meta.len();
            *count += 1;
        }
    }
}

// ─── Safety helpers ─────────────────────────────────────────────────────────

/// 校验 TEMP / TMP 是否是合理的"临时目录"：
/// 1) 末段名必须是 `temp` 或 `tmp`（不区分大小写）
/// 2) 路径深度至少 2 级（盘符 + 至少一个目录），避免误指向 `D:\`
/// 3) 不能等于一些关键目录（home / Desktop / Documents / Windows / Program Files 等）
///
/// 即便用户自己把 TEMP 改到了 `C:\Users\xxx` 这种危险位置，也不会被清理。
fn validate_temp_dir(p: &Path) -> Option<PathBuf> {
    if p.as_os_str().is_empty() || !p.exists() {
        return None;
    }
    let canonical = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());

    // 1) 末段必须是 temp/tmp
    let last_seg_ok = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.eq_ignore_ascii_case("temp") || s.eq_ignore_ascii_case("tmp"))
        .unwrap_or(false);
    if !last_seg_ok {
        return None;
    }

    // 2) 至少要有 2 个组件（Windows 上前缀 `\\?\` 也算，所以这里用 >=2 已足够保守）
    let depth = canonical
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .count();
    if depth < 2 {
        return None;
    }

    // 3) 黑名单：不能等于这些关键目录
    let bad_dirs: Vec<PathBuf> = [
        dirs::home_dir(),
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::data_local_dir(),
        dirs::data_dir(),
        Some(PathBuf::from(r"C:\")),
        Some(PathBuf::from(r"C:\Windows")),
        Some(PathBuf::from(r"C:\Windows\System32")),
        Some(PathBuf::from(r"C:\Program Files")),
        Some(PathBuf::from(r"C:\Program Files (x86)")),
        Some(PathBuf::from(r"C:\Users")),
    ]
    .into_iter()
    .flatten()
    .collect();

    let canon_norm = canonical.to_string_lossy().to_lowercase();
    for bad in &bad_dirs {
        if canon_norm == bad.to_string_lossy().to_lowercase() {
            return None;
        }
    }

    Some(canonical)
}

/// 统计 conda pkgs 目录下的压缩包文件大小（*.tar.bz2 / *.conda）
fn conda_archive_size(pkgs_dir: &Path) -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;
    let Ok(entries) = std::fs::read_dir(pkgs_dir) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_conda_archive(&name_str) {
            size += meta.len();
            count += 1;
        }
    }
    (size, count)
}

fn is_conda_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".tar.bz2") || lower.ends_with(".conda")
}

/// 仅删除 conda pkgs 下的压缩包文件，保留解压后的目录（环境硬链接依赖）
fn remove_conda_archives_with_progress(
    pkgs_dir: &Path,
    progress: &mut CleanProgress<'_>,
) -> std::io::Result<(u64, u64)> {
    let mut freed = 0u64;
    let mut count = 0u64;
    for entry in std::fs::read_dir(pkgs_dir)?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_conda_archive(&name_str) {
            continue;
        }
        let sz = meta.len();
        if std::fs::remove_file(entry.path()).is_ok() {
            freed += sz;
            count += 1;
            progress.add_result(1, sz, 1);
        } else {
            progress.skip(1);
        }
    }
    Ok((freed, count))
}

/// 扫描缩略图/图标缓存（仅 thumbcache_*.db / iconcache_*.db）
fn scan_thumbnail_cache(dir: &Path) -> Option<CleanupCategory> {
    if !dir.exists() {
        return None;
    }
    let (size, count) = thumbnail_files_size(dir);
    if size == 0 {
        return None;
    }
    Some(CleanupCategory {
        id: "thumbnails".to_string(),
        name: "缩略图缓存".to_string(),
        description: "Windows 资源管理器缩略图与图标缓存（thumbcache_*.db / iconcache_*.db）"
            .to_string(),
        size_bytes: size,
        file_count: count,
        paths: vec![PathDetail {
            path: dir.to_string_lossy().to_string(),
            size_bytes: size,
            file_count: count,
        }],
    })
}

fn is_thumbnail_cache_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    (lower.starts_with("thumbcache_") || lower.starts_with("iconcache_")) && lower.ends_with(".db")
}

fn thumbnail_files_size(dir: &Path) -> (u64, u64) {
    let mut size = 0u64;
    let mut count = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_thumbnail_cache_file(&name_str) {
            size += meta.len();
            count += 1;
        }
    }
    (size, count)
}

/// 仅删除 thumbcache_*.db / iconcache_*.db 文件，保留目录下其他系统文件
/// （UsrClass.dat、shellbags、Quick Access 等）
fn remove_thumbnail_files_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
) -> std::io::Result<(u64, u64)> {
    let mut freed = 0u64;
    let mut count = 0u64;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_thumbnail_cache_file(&name_str) {
            continue;
        }
        let size = meta.len();
        if std::fs::remove_file(entry.path()).is_ok() {
            freed += size;
            count += 1;
            progress.add_result(1, size, 1);
        } else {
            progress.skip(1);
        }
    }
    Ok((freed, count))
}

/// 通过 Win32 `SHEmptyRecycleBinW` 清空所有盘符的回收站。
/// 直接 `remove_dir_all C:\$Recycle.Bin\<SID>` 会破坏回收站结构，
/// 必须由 Shell API 完成。
fn empty_recycle_bin() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{
        SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND,
    };

    let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
    // pszrootpath = NULL → 清空所有盘符的回收站
    let result = unsafe { SHEmptyRecycleBinW(HWND(std::ptr::null_mut()), PCWSTR::null(), flags) };
    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(format!("SHEmptyRecycleBinW failed: {}", e)),
    }
}

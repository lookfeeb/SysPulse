use crate::error::IpcError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static SCANNING: AtomicBool = AtomicBool::new(false);

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
    if SCANNING.swap(true, Ordering::SeqCst) {
        return Err(crate::error::AppError::Invalid("scan already in progress".into()).into());
    }
    let result = tokio::task::spawn_blocking(do_scan).await.unwrap();
    SCANNING.store(false, Ordering::Relaxed);
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn clean_categories(args: CleanArgs) -> Result<CleanResult, IpcError> {
    let ids = args.category_ids;
    let excluded = args.excluded_paths;
    let result = tokio::task::spawn_blocking(move || do_clean(&ids, &excluded)).await.unwrap();
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn scan_large_files(args: LargeFileScanArgs) -> Result<LargeFileScanResult, IpcError> {
    let result = tokio::task::spawn_blocking(move || do_scan_large_files(&args)).await.unwrap();
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
            std::env::var("TEMP").ok().and_then(|s| validate_temp_dir(Path::new(&s))),
            std::env::var("TMP").ok().and_then(|s| validate_temp_dir(Path::new(&s))),
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
        &[Some(PathBuf::from(r"C:\Windows\SoftwareDistribution\Download"))],
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

    // 11. Office cache
    if let Some(local) = dirs::data_local_dir() {
        if let Some(cat) = scan_dir_category(
            "office-cache",
            "Office 缓存",
            "Microsoft Office 文件缓存",
            &[
                Some(local.join("Microsoft").join("Office").join("16.0").join("OfficeFileCache")),
            ],
        ) {
            categories.push(cat);
        }
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
    let browser_caches = [
        local.join("Google").join("Chrome").join("User Data").join("Default").join("Cache"),
        local.join("Google").join("Chrome").join("User Data").join("Default").join("Code Cache"),
        local.join("Microsoft").join("Edge").join("User Data").join("Default").join("Cache"),
        local.join("Microsoft").join("Edge").join("User Data").join("Default").join("Code Cache"),
        local.join("Mozilla").join("Firefox").join("Profiles"),
    ];

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
        description: "Chrome / Edge / Firefox 缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
    })
}

// ─── Clean logic ────────────────────────────────────────────────────────────

fn do_clean(category_ids: &[String], excluded_paths: &[String]) -> CleanResult {
    let scan = do_scan();
    let mut freed = 0u64;
    let mut deleted = 0u64;
    let mut errors = Vec::new();
    let excluded_set: std::collections::HashSet<&str> = excluded_paths.iter().map(|s| s.as_str()).collect();

    for cat in &scan.categories {
        if !category_ids.contains(&cat.id) {
            continue;
        }

        match cat.id.as_str() {
            // 回收站：必须走 SHEmptyRecycleBinW，直接遍历删除会破坏 $Recycle.Bin 结构
            "recycle-bin" => match empty_recycle_bin() {
                Ok(()) => {
                    freed += cat.size_bytes;
                    deleted += cat.file_count;
                }
                Err(e) => errors.push(format!("回收站: {}", e)),
            },

            // 缩略图：只删 thumbcache_*.db / iconcache_*.db，不能整目录清空
            "thumbnails" => {
                for detail in &cat.paths {
                    if excluded_set.contains(detail.path.as_str()) {
                        continue;
                    }
                    let path = Path::new(&detail.path);
                    if !path.exists() {
                        continue;
                    }
                    match remove_thumbnail_files(path) {
                        Ok((s, c)) => {
                            freed += s;
                            deleted += c;
                        }
                        Err(e) => errors.push(format!("{}: {}", detail.path, e)),
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
                        continue;
                    }
                    let is_conda_pkgs = path.ends_with("pkgs")
                        && path.to_string_lossy().to_lowercase().contains("conda");
                    let result = if is_conda_pkgs {
                        remove_conda_archives(path)
                    } else {
                        remove_dir_contents(path)
                    };
                    match result {
                        Ok((s, c)) => {
                            freed += s;
                            deleted += c;
                        }
                        Err(e) => errors.push(format!("{}: {}", detail.path, e)),
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
                        continue;
                    }
                    match remove_dir_contents(path) {
                        Ok((s, c)) => {
                            freed += s;
                            deleted += c;
                        }
                        Err(e) => errors.push(format!("{}: {}", detail.path, e)),
                    }
                }
            }
        }
    }

    CleanResult {
        freed_bytes: freed,
        deleted_files: deleted,
        errors,
    }
}

fn remove_dir_contents(dir: &Path) -> std::io::Result<(u64, u64)> {
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
            let (s, c) = dir_size(&path);
            if std::fs::remove_dir_all(&path).is_ok() {
                freed += s;
                count += c;
            }
        } else {
            let size = meta.len();
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
                count += 1;
            }
        }
    }

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

    scan_large_recursive(root, min_bytes, limit, &mut files, &mut total_scanned, 0, 20);
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
            scan_large_recursive(&path, min_bytes, limit, results, total_scanned, depth + 1, max_depth);
        } else {
            *total_scanned += 1;
            let size = meta.len();
            if size >= min_bytes {
                results.push(LargeFile {
                    path: path.to_string_lossy().to_string(),
                    size_bytes: size,
                });
            }
        }
    }
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
        if !meta.is_file() { continue; }
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
fn remove_conda_archives(pkgs_dir: &Path) -> std::io::Result<(u64, u64)> {
    let mut freed = 0u64;
    let mut count = 0u64;
    for entry in std::fs::read_dir(pkgs_dir)?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() { continue; }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !is_conda_archive(&name_str) { continue; }
        let sz = meta.len();
        if std::fs::remove_file(entry.path()).is_ok() {
            freed += sz;
            count += 1;
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
        description: "Windows 资源管理器缩略图与图标缓存（thumbcache_*.db / iconcache_*.db）".to_string(),
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
fn remove_thumbnail_files(dir: &Path) -> std::io::Result<(u64, u64)> {
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

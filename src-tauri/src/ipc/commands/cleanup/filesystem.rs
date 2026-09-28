use super::*;

pub(super) fn dir_size(path: &Path) -> (u64, u64) {
    dir_size_with_min_age(path, None)
}

pub(super) fn dir_size_with_min_age(path: &Path, min_age_days: Option<u32>) -> (u64, u64) {
    schedule_dir_size(path, min_age_days).wait()
}

pub(super) fn schedule_dir_size(path: &Path, min_age_days: Option<u32>) -> DirectoryMeasureCell {
    if let Some(measures) = current_scan_directory_measures() {
        let key = DirectoryMeasureKey {
            path: normalize_path_key(&path.to_string_lossy()),
            min_age_days,
        };
        let (measurement, should_schedule) = match measures.lock() {
            Ok(mut cached) => match cached.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => (entry.get().clone(), false),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let measurement = Arc::new(DirectoryMeasureCellState::default());
                    entry.insert(measurement.clone());
                    (measurement, true)
                }
            },
            Err(_) => return schedule_dir_size_uncached(path, min_age_days),
        };
        if should_schedule && !queue_dir_size(path, min_age_days, measurement.clone()) {
            measurement.complete(dir_size_with_min_age_inline(path, min_age_days));
        }
        return measurement;
    }
    schedule_dir_size_uncached(path, min_age_days)
}

pub(super) fn schedule_dir_size_uncached(
    path: &Path,
    min_age_days: Option<u32>,
) -> DirectoryMeasureCell {
    let measurement = Arc::new(DirectoryMeasureCellState::default());
    if !queue_dir_size(path, min_age_days, measurement.clone()) {
        measurement.complete(dir_size_with_min_age_inline(path, min_age_days));
    }
    measurement
}

pub(super) fn dir_size_with_min_age_inline(path: &Path, min_age_days: Option<u32>) -> (u64, u64) {
    let cutoff = min_age_days.and_then(|days| {
        SystemTime::now().checked_sub(Duration::from_secs(days as u64 * 24 * 60 * 60))
    });
    let mut result = DirectoryMeasure::default();
    let mut local_visited_files = std::collections::HashSet::new();
    dir_size_recursive(
        path,
        &mut result,
        &mut local_visited_files,
        0,
        DIRECTORY_SCAN_MAX_DEPTH,
        cutoff,
    );
    with_scan_diagnostics(|diagnostics| {
        for warning in result.warnings {
            diagnostics.warn(warning);
        }
        diagnostics.progress("engineer", "工兵蚁正在统计目录", Some(path));
    });
    (result.size, result.count)
}

#[derive(Default)]
pub(super) struct DirectoryMeasure {
    pub(super) size: u64,
    pub(super) count: u64,
    pub(super) warnings: Vec<String>,
    pub(super) stopped: bool,
}

pub(super) fn dir_size_recursive(
    path: &Path,
    result: &mut DirectoryMeasure,
    local_visited_files: &mut std::collections::HashSet<FileIdentity>,
    depth: u32,
    max_depth: u32,
    cutoff: Option<SystemTime>,
) {
    if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed) {
        result.stopped = true;
        return;
    }
    if let Some(diagnostics) = current_scan_diagnostics() {
        if !diagnostics.should_continue(path) {
            result.stopped = true;
            return;
        }
        let dispatched = diagnostics.dispatch_engineer();
        if dispatched == 1 || dispatched % 512 == 0 {
            diagnostics.progress("engineer", "工兵蚁营正在建立目录数据", Some(path));
        }
    }
    if depth > max_depth {
        push_measure_warning(
            result,
            format!("{}: 超过最大扫描深度 {}", path.display(), max_depth),
        );
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            push_measure_warning(
                result,
                format!("{}: 读取目录失败: {}", path.display(), error),
            );
            return;
        }
    };
    for entry in entries {
        if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed) {
            result.stopped = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_measure_warning(
                    result,
                    format!("{}: 读取目录项失败: {}", path.display(), error),
                );
                continue;
            }
        };
        let entry_path = entry.path();
        if let Some(diagnostics) = current_scan_diagnostics() {
            if !diagnostics.try_visit(&entry_path) {
                result.stopped = true;
                break;
            }
        }
        let meta = match std::fs::symlink_metadata(&entry_path) {
            Ok(meta) => meta,
            Err(error) => {
                push_measure_warning(
                    result,
                    format!("{}: 读取元数据失败: {}", entry_path.display(), error),
                );
                continue;
            }
        };
        if metadata_is_reparse_point(&meta) {
            with_scan_diagnostics(|diagnostics| diagnostics.skip_expected(1));
            continue;
        }
        if meta.is_dir() {
            dir_size_recursive(
                &entry_path,
                result,
                local_visited_files,
                depth + 1,
                max_depth,
                cutoff,
            );
            if result.stopped {
                break;
            }
        } else {
            if cutoff
                .is_some_and(|cutoff| meta.modified().map_or(true, |modified| modified > cutoff))
            {
                continue;
            }
            if let Some((volume_serial, file_id, allocated_size)) = file_details(&entry_path, &meta)
            {
                let identity = (volume_serial, file_id);
                if !local_visited_files.insert(identity) {
                    continue;
                }
                result.size = result.size.saturating_add(allocated_size);
            } else {
                result.size = result.size.saturating_add(meta.len());
            }
            result.count = result.count.saturating_add(1);
        }
    }
}

pub(super) fn push_measure_warning(result: &mut DirectoryMeasure, warning: String) {
    if result.warnings.len() < MAX_SCAN_WARNINGS {
        result.warnings.push(warning);
    }
}

pub(super) fn safe_cleanup_root(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() || metadata_is_reparse_point(&meta) {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

pub(super) fn path_detail_identity(path: &Path) -> PathDetail {
    let identity = directory_identity(path);
    PathDetail {
        path: String::new(),
        size_bytes: 0,
        file_count: 0,
        matched_rule: String::new(),
        source: String::new(),
        volume_serial: identity.map(|value| value.0),
        file_id: identity.map(|value| value.1),
    }
}

pub(super) fn revalidate_cleanup_root(expected: &PathDetail) -> Option<PathBuf> {
    let path = Path::new(&expected.path);
    let canonical = safe_cleanup_root(path)?;
    match (expected.volume_serial, expected.file_id) {
        (Some(volume_serial), Some(file_id)) => {
            let current = directory_identity(&canonical)?;
            (current == (volume_serial, file_id)).then_some(canonical)
        }
        _ => None,
    }
}

#[cfg(windows)]
pub(super) fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    file_handle_identity(path, true).map(|value| (value.0, value.1))
}

#[cfg(windows)]
pub(super) fn file_details(path: &Path, _metadata: &std::fs::Metadata) -> Option<(u64, u64, u64)> {
    file_handle_identity(path, false)
}

#[cfg(windows)]
pub(super) fn file_handle_identity(path: &Path, directory: bool) -> Option<(u64, u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileStandardInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, OPEN_EXISTING,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let flags = if directory {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_FLAGS_AND_ATTRIBUTES(0)
    };
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            HANDLE::default(),
        )
        .ok()?
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let identity_result = unsafe { GetFileInformationByHandle(handle, &mut info) };
    let mut standard = FILE_STANDARD_INFO::default();
    let standard_result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            &mut standard as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    identity_result.ok()?;
    let file_id = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
    let allocated = standard_result
        .ok()
        .map(|_| standard.AllocationSize.max(0) as u64)
        .unwrap_or(((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64);
    Some((info.dwVolumeSerialNumber as u64, file_id, allocated))
}

#[cfg(not(windows))]
pub(super) fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(windows))]
pub(super) fn file_details(_path: &Path, metadata: &std::fs::Metadata) -> Option<(u64, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((
        metadata.dev(),
        metadata.ino(),
        metadata.blocks().saturating_mul(512),
    ))
}

pub(super) fn metadata_is_reparse_point(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

// ─── Safety helpers ─────────────────────────────────────────────────────────

/// 校验 TEMP / TMP 是否是合理的"临时目录"：
/// 1) 末段名必须是 `temp` 或 `tmp`（不区分大小写）
/// 2) 路径深度至少 2 级（盘符 + 至少一个目录），避免误指向 `D:\`
/// 3) 不能等于一些关键目录（home / Desktop / Documents / Windows / Program Files 等）
///
/// 即便用户自己把 TEMP 改到了 `C:\Users\xxx` 这种危险位置，也不会被清理。
pub(super) fn validate_temp_dir(p: &Path) -> Option<PathBuf> {
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
pub(super) fn conda_archive_size(pkgs_dir: &Path) -> (u64, u64) {
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

pub(super) fn is_conda_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".tar.bz2") || lower.ends_with(".conda")
}

/// 仅删除 conda pkgs 下的压缩包文件，保留解压后的目录（环境硬链接依赖）
pub(super) fn remove_conda_archives_with_progress(
    pkgs_dir: &Path,
    progress: &mut CleanProgress<'_>,
    errors: &mut Vec<String>,
    file_policy: CleanupFilePolicy,
    diagnostics: &mut PathCleanupDiagnostics,
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
        let entry_path = entry.path();
        let decision = file_policy.file_decision(&meta);
        if decision != CleanupPolicyDecision::Allow {
            diagnostics.record(&entry_path, decision);
            continue;
        }
        let sz = file_details(&entry_path, &meta)
            .map(|value| value.2)
            .unwrap_or_else(|| meta.len());
        if std::fs::remove_file(&entry_path).is_ok() {
            freed += sz;
            count += 1;
            progress.add_result(1, sz, 1);
        } else {
            errors.push(format!("{}: 删除文件失败", entry_path.display()));
            progress.fail(1);
        }
    }
    Ok((freed, count))
}

/// 扫描缩略图/图标缓存（仅 thumbcache_*.db / iconcache_*.db）
pub(super) fn scan_thumbnail_cache(dir: &Path) -> Option<CleanupCategory> {
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
            matched_rule: category_matched_rule("thumbnails").to_string(),
            source: path_source("thumbnails", dir).to_string(),
            ..path_detail_identity(dir)
        }],
        risk_level: cleanup_policy("thumbnails").risk,
        default_selected: cleanup_policy("thumbnails").default_selected,
        min_age_days: cleanup_policy("thumbnails").min_age_days,
    })
}

pub(super) fn is_thumbnail_cache_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    (lower.starts_with("thumbcache_") || lower.starts_with("iconcache_")) && lower.ends_with(".db")
}

pub(super) fn thumbnail_files_size(dir: &Path) -> (u64, u64) {
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
pub(super) fn remove_thumbnail_files_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    errors: &mut Vec<String>,
    file_policy: CleanupFilePolicy,
    diagnostics: &mut PathCleanupDiagnostics,
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
        let entry_path = entry.path();
        let decision = file_policy.file_decision(&meta);
        if decision != CleanupPolicyDecision::Allow {
            diagnostics.record(&entry_path, decision);
            continue;
        }
        let size = file_details(&entry_path, &meta)
            .map(|value| value.2)
            .unwrap_or_else(|| meta.len());
        if std::fs::remove_file(&entry_path).is_ok() {
            freed += size;
            count += 1;
            progress.add_result(1, size, 1);
        } else {
            errors.push(format!("{}: 删除文件失败", entry_path.display()));
            progress.fail(1);
        }
    }
    Ok((freed, count))
}

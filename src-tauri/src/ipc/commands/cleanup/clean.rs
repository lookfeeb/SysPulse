use super::*;

// ─── Clean logic ────────────────────────────────────────────────────────────

pub(super) struct CleanProgress<'a> {
    pub(super) app: Option<&'a AppHandle>,
    pub(super) total_items: u64,
    pub(super) processed_items: u64,
    pub(super) current_category: String,
    pub(super) current_path: Option<String>,
    pub(super) freed_bytes: u64,
    pub(super) deleted_files: u64,
    pub(super) failed_items: u64,
    pub(super) last_emit: Instant,
}

#[derive(Clone, Copy)]
pub(super) struct CleanupFilePolicy {
    pub(super) min_age_days: Option<u32>,
    pub(super) scanned_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CleanupPolicyDecision {
    Allow,
    NotEligibleAtScan,
    ChangedAfterScan,
    MetadataUnavailable,
}

#[derive(Default)]
pub(super) struct PathCleanupDiagnostics {
    pub(super) changed_entries: u64,
    pub(super) unverifiable_entries: u64,
    pub(super) examples: Vec<String>,
}

impl PathCleanupDiagnostics {
    pub(super) fn record(&mut self, path: &Path, decision: CleanupPolicyDecision) {
        match decision {
            CleanupPolicyDecision::ChangedAfterScan => {
                self.changed_entries = self.changed_entries.saturating_add(1);
            }
            CleanupPolicyDecision::MetadataUnavailable => {
                self.unverifiable_entries = self.unverifiable_entries.saturating_add(1);
            }
            CleanupPolicyDecision::Allow | CleanupPolicyDecision::NotEligibleAtScan => return,
        }
        if self.examples.len() < 3 {
            self.examples.push(path.to_string_lossy().to_string());
        }
    }

    pub(super) fn has_safety_skips(&self) -> bool {
        self.changed_entries > 0 || self.unverifiable_entries > 0
    }
}

impl CleanupFilePolicy {
    pub(super) fn for_category(category: &CleanupCategory, scan: &ScanResult) -> Self {
        Self {
            min_age_days: category.min_age_days,
            scanned_at: scan_timestamp(scan.scanned_at_ms),
        }
    }

    pub(super) fn file_decision(&self, metadata: &std::fs::Metadata) -> CleanupPolicyDecision {
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(_) => return CleanupPolicyDecision::MetadataUnavailable,
        };
        if let Some(scanned_at) = self.scanned_at {
            if modified > scanned_at || metadata.created().is_ok_and(|created| created > scanned_at)
            {
                return CleanupPolicyDecision::ChangedAfterScan;
            }
        }
        let age_reference = self.scanned_at.unwrap_or_else(SystemTime::now);
        if let Some(cutoff) = self.min_age_days.and_then(|days| {
            age_reference.checked_sub(Duration::from_secs(days as u64 * 24 * 60 * 60))
        }) {
            if modified > cutoff {
                return CleanupPolicyDecision::NotEligibleAtScan;
            }
        }
        CleanupPolicyDecision::Allow
    }

    pub(super) fn directory_decision(&self, metadata: &std::fs::Metadata) -> CleanupPolicyDecision {
        let Some(scanned_at) = self.scanned_at else {
            return CleanupPolicyDecision::Allow;
        };
        match metadata.modified() {
            Ok(modified)
                if modified <= scanned_at
                    && !metadata.created().is_ok_and(|created| created > scanned_at) =>
            {
                CleanupPolicyDecision::Allow
            }
            Ok(_) => CleanupPolicyDecision::ChangedAfterScan,
            Err(_) => CleanupPolicyDecision::MetadataUnavailable,
        }
    }
}

pub(super) fn scan_timestamp(scanned_at_ms: i64) -> Option<SystemTime> {
    (scanned_at_ms > 0)
        .then(|| SystemTime::UNIX_EPOCH + Duration::from_millis(scanned_at_ms as u64))
}

impl<'a> CleanProgress<'a> {
    pub(super) fn new(app: &'a AppHandle, total_items: u64) -> Self {
        Self {
            app: Some(app),
            total_items,
            processed_items: 0,
            current_category: String::new(),
            current_path: None,
            freed_bytes: 0,
            deleted_files: 0,
            failed_items: 0,
            last_emit: Instant::now() - Duration::from_millis(250),
        }
    }

    pub(super) fn set_current(&mut self, category: &str, path: Option<&str>) {
        self.current_category = category.to_string();
        self.current_path = path.map(|p| p.to_string());
        self.emit(false, false);
    }

    pub(super) fn add_result(&mut self, processed: u64, freed: u64, deleted: u64) {
        self.processed_items = self.processed_items.saturating_add(processed);
        self.freed_bytes = self.freed_bytes.saturating_add(freed);
        self.deleted_files = self.deleted_files.saturating_add(deleted);
        self.emit(false, false);
    }

    pub(super) fn skip(&mut self, processed: u64) {
        self.processed_items = self.processed_items.saturating_add(processed);
        self.emit(false, false);
    }

    pub(super) fn fail(&mut self, processed: u64) {
        self.failed_items = self.failed_items.saturating_add(processed);
        self.skip(processed);
    }

    pub(super) fn finish(&mut self) {
        self.emit(true, true);
    }

    pub(super) fn emit(&mut self, force: bool, done: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_millis(120) {
            return;
        }
        let percent = self
            .processed_items
            .saturating_mul(100)
            .checked_div(self.total_items)
            .map(|value| value.min(if done { 100 } else { 99 }) as u8)
            .unwrap_or(if done { 100 } else { 0 });
        if let Some(app) = self.app {
            let _ = app.emit(
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
        }
        self.last_emit = Instant::now();
    }
}

pub(super) fn remaining_path_items(
    processed_before: u64,
    processed_after: u64,
    expected_items: u64,
) -> u64 {
    let processed_for_path = processed_after
        .saturating_sub(processed_before)
        .min(expected_items);
    expected_items.saturating_sub(processed_for_path)
}

pub(super) fn settle_path_progress(
    progress: &mut CleanProgress<'_>,
    processed_before: u64,
    expected_items: u64,
) -> u64 {
    let remaining =
        remaining_path_items(processed_before, progress.processed_items, expected_items);
    if remaining > 0 {
        progress.skip(remaining);
    }
    remaining
}

pub(super) fn append_path_cleanup_diagnostics(
    root: &str,
    diagnostics: &PathCleanupDiagnostics,
    unprocessed_items: u64,
    operation_failed: bool,
    errors: &mut Vec<String>,
) {
    if diagnostics.has_safety_skips() {
        let examples = if diagnostics.examples.is_empty() {
            String::new()
        } else {
            format!("；示例: {}", diagnostics.examples.join("、"))
        };
        errors.push(format!(
            "{root}: 扫描后检测到 {} 个变化项、{} 个无法复验项，已安全跳过；本路径有 {} 个扫描项未处理{examples}，请重新扫描",
            diagnostics.changed_entries, diagnostics.unverifiable_entries, unprocessed_items
        ));
    } else if unprocessed_items > 0 && !operation_failed {
        errors.push(format!(
            "{root}: 扫描后内容发生变化、项目已不存在或无法复验，已跳过 {unprocessed_items} 个扫描项，请重新扫描"
        ));
    }
}

pub(super) fn do_clean(
    app: &AppHandle,
    scan: ScanResult,
    category_ids: &[String],
    excluded_paths: &[String],
) -> CleanResult {
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
    let running_processes = if selected_categories
        .iter()
        .any(|category| watched_processes(&category.id).is_some())
    {
        running_process_snapshot()
    } else {
        Vec::new()
    };
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
        let file_policy = CleanupFilePolicy::for_category(cat, &scan);
        for detail in &cat.paths {
            if excluded_set.contains(detail.path.as_str()) {
                continue;
            }
            let blockers =
                running_process_blockers_for_path(&cat.id, &detail.path, &running_processes);
            if !blockers.is_empty() {
                errors.push(format!(
                    "{} 已跳过，正在被以下程序使用: {}",
                    detail.path,
                    blockers.join(", ")
                ));
                progress.skip(detail.file_count.max(1));
                continue;
            }
            let Some(path) = revalidate_cleanup_root(detail) else {
                errors.push(format!("路径安全校验失败: {}", detail.path));
                progress.skip(detail.file_count.max(1));
                continue;
            };
            progress.set_current(&cat.name, Some(&detail.path));
            let processed_before = progress.processed_items;
            let expected_items = detail.file_count.max(1);
            let mut diagnostics = PathCleanupDiagnostics::default();
            // 特殊类别只删除指定文件，其他类别保留根目录并清理内容。
            let result = match cat.id.as_str() {
                "thumbnails" => remove_thumbnail_files_with_progress(
                    &path,
                    &mut progress,
                    &mut errors,
                    file_policy,
                    &mut diagnostics,
                ),
                "python-cache"
                    if path.ends_with("pkgs")
                        && path.to_string_lossy().to_lowercase().contains("conda") =>
                {
                    remove_conda_archives_with_progress(
                        &path,
                        &mut progress,
                        &mut errors,
                        file_policy,
                        &mut diagnostics,
                    )
                }
                _ => remove_dir_contents_with_progress(
                    &path,
                    &mut progress,
                    file_policy,
                    &mut errors,
                    &mut diagnostics,
                ),
            };
            let operation_failed = result.is_err();
            if let Err(error) = result {
                errors.push(format!("{}: {}", detail.path, error));
            }
            let remaining = settle_path_progress(&mut progress, processed_before, expected_items);
            append_path_cleanup_diagnostics(
                &detail.path,
                &diagnostics,
                remaining,
                operation_failed,
                &mut errors,
            );
        }
    }

    progress.finish();
    CleanResult {
        freed_bytes: progress.freed_bytes,
        deleted_files: progress.deleted_files,
        errors,
    }
}

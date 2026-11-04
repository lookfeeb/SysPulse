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

pub(super) fn scan_timestamp(scanned_at_ms: i64) -> Option<SystemTime> {
    (scanned_at_ms > 0)
        .then(|| SystemTime::UNIX_EPOCH + Duration::from_millis(scanned_at_ms as u64))
}

impl<'a> CleanProgress<'a> {
    pub(super) fn new(app: Option<&'a AppHandle>, total_items: u64) -> Self {
        Self {
            app,
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

/// Execute the immutable scan inventory. Directory timestamps never determine
/// eligibility; every file is independently validated on its delete handle.
pub(super) fn do_clean(
    app: Option<&AppHandle>,
    scan: ScanResult,
    category_ids: &[String],
    excluded_paths: &[String],
) -> CleanResult {
    let plan = scan
        .plan
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let selected: Vec<_> = scan
        .categories
        .iter()
        .filter(|cat| category_ids.contains(&cat.id))
        .flat_map(|cat| {
            cat.paths
                .iter()
                .filter(|path| !excluded_paths.contains(&path.path))
                .map(move |path| (cat, path))
        })
        .collect();
    let excluded_keys: Vec<_> = excluded_paths
        .iter()
        .map(|path| normalize_path_key(path))
        .collect();
    let eligible = |category: &CleanupCategory, root: &PathDetail, file: &PlannedFile| {
        let key = normalize_path_key(&file.path.to_string_lossy());
        if excluded_keys
            .iter()
            .any(|excluded| path_is_within_key(&key, excluded))
        {
            return false;
        }
        let name = file.path.file_name().unwrap_or_default().to_string_lossy();
        let direct = file
            .path
            .parent()
            .is_some_and(|parent| same_path(parent, Path::new(&root.path)));
        if category.id == "thumbnails" && (!direct || !is_thumbnail_cache_file(&name)) {
            return false;
        }
        if category.id == "python-cache"
            && Path::new(&root.path).ends_with("pkgs")
            && (!direct || !is_conda_archive(&name))
        {
            return false;
        }
        if let Some(days) = category.min_age_days {
            let Some(cutoff) = scan_timestamp(scan.scanned_at_ms)
                .and_then(|time| time.checked_sub(Duration::from_secs(u64::from(days) * 86400)))
            else {
                return false;
            };
            if file.modified > cutoff {
                return false;
            }
        }
        true
    };
    let total = selected
        .iter()
        .flat_map(|(category, root)| {
            plan.under(Path::new(&root.path))
                .filter(|file| eligible(category, root, file))
                .map(|file| &file.path)
        })
        .collect::<std::collections::HashSet<_>>()
        .len() as u64;
    let mut progress = CleanProgress::new(app, total);
    let mut reports = Vec::new();
    let mut processed = std::collections::HashSet::new();
    let processes = running_process_snapshot();
    for (category, root) in selected {
        let mut report = CleanPathReport {
            path: display_cleanup_path(&root.path),
            category: category.name.clone(),
            ..Default::default()
        };
        let blockers = running_process_blockers_for_path(&category.id, &root.path, &processes);
        for file in plan.under(Path::new(&root.path)) {
            if !eligible(category, root, file) || !processed.insert(&file.path) {
                continue;
            }
            progress.set_current(
                &category.name,
                Some(&display_cleanup_path(&file.path.to_string_lossy())),
            );
            if CLEAN_CANCEL_REQUESTED.load(Ordering::Relaxed) {
                report.cancelled_files += 1;
                progress.skip(1);
                continue;
            }
            if !blockers.is_empty() {
                report.busy_files += 1;
                progress.skip(1);
                continue;
            }
            match delete_planned_file(root, file) {
                Ok(DeleteOutcome::Deleted(bytes)) => {
                    report.deleted_files += 1;
                    report.freed_bytes += bytes;
                    progress.add_result(1, bytes, 1);
                }
                Ok(DeleteOutcome::Changed) => {
                    report.changed_files += 1;
                    progress.skip(1);
                }
                Ok(DeleteOutcome::Missing) => {
                    report.missing_files += 1;
                    progress.skip(1);
                }
                Err(error) => {
                    if matches!(error.raw_os_error(), Some(32 | 33)) {
                        report.busy_files += 1;
                    } else {
                        report.failed_files += 1;
                    }
                    if report.examples.len() < 3 {
                        report.examples.push(format!(
                            "{}: {}",
                            display_cleanup_path(&file.path.to_string_lossy()),
                            error
                        ));
                    }
                    progress.fail(1);
                }
            }
        }
        if !blockers.is_empty() {
            report
                .examples
                .push(format!("正在使用：{}", blockers.join("、")));
        }
        reports.push(report);
    }
    progress.finish();
    let skipped_files = reports
        .iter()
        .map(|r| {
            r.changed_files + r.missing_files + r.busy_files + r.failed_files + r.cancelled_files
        })
        .sum();
    let errors = reports
        .iter()
        .filter(|r| r.failed_files > 0)
        .map(|r| format!("{}：{} 个文件未能清理", r.path, r.failed_files))
        .collect();
    CleanResult {
        freed_bytes: progress.freed_bytes,
        deleted_files: progress.deleted_files,
        skipped_files,
        cancelled: CLEAN_CANCEL_REQUESTED.load(Ordering::SeqCst),
        paths: reports,
        errors,
    }
}

pub(super) fn display_cleanup_path(path: &str) -> String {
    path.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| path.strip_prefix(r"\\?\").unwrap_or(path).to_string())
}

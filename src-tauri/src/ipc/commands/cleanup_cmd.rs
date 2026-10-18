use crate::error::IpcError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

#[path = "cleanup/runtime.rs"]
mod runtime;
use runtime::*;

#[path = "cleanup/scan.rs"]
mod scan;
use scan::*;

#[path = "cleanup/tool_caches.rs"]
mod tool_caches;
use tool_caches::*;

#[path = "cleanup/discovery.rs"]
mod discovery;
use discovery::*;

#[path = "cleanup/system_caches.rs"]
mod system_caches;
use system_caches::*;

#[path = "cleanup/clean.rs"]
mod clean;
use clean::*;

#[path = "cleanup/remove.rs"]
mod remove;
use remove::*;

#[path = "cleanup/large_files.rs"]
mod large_files;
use large_files::*;

#[path = "cleanup/filesystem.rs"]
mod filesystem;
use filesystem::*;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PathDetail {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub matched_rule: String,
    pub source: String,
    pub volume_serial: Option<u64>,
    pub file_id: Option<u64>,
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
    pub risk_level: CleanupRisk,
    pub default_selected: bool,
    pub min_age_days: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupRisk {
    Safe,
    Caution,
    Advanced,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub scan_id: String,
    pub categories: Vec<CleanupCategory>,
    pub total_size_bytes: u64,
    pub total_file_count: u64,
    pub scanned_at_ms: i64,
    pub duration_ms: u64,
    pub scanned_paths: u64,
    pub skipped_paths: u64,
    pub ignored_paths: u64,
    pub hotspot_count: u64,
    pub scout_workers: u32,
    pub engineer_workers: u32,
    pub scout_tasks: u64,
    pub engineer_tasks: u64,
    pub project_roots: Vec<String>,
    pub warnings: Vec<String>,
    pub complete: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanupScanProgressEvent {
    pub stage: String,
    pub phase: String,
    pub scanned_paths: u64,
    pub skipped_paths: u64,
    pub ignored_paths: u64,
    pub scout_tasks: u64,
    pub engineer_tasks: u64,
    pub current_path: Option<String>,
    pub done: bool,
    pub cancelled: bool,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCleanupArgs {
    pub scan_id: String,
    pub path: String,
    pub selected_category_ids: Vec<String>,
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportCleanupResult {
    pub saved_to: String,
    pub records: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanupExportDocument {
    schema_version: u32,
    scan_id: String,
    scanned_at_ms: i64,
    duration_ms: u64,
    complete: bool,
    cancelled: bool,
    warnings: Vec<String>,
    project_roots: Vec<String>,
    hotspot_count: u64,
    scout_workers: u32,
    engineer_workers: u32,
    scout_tasks: u64,
    engineer_tasks: u64,
    selected_total_size_bytes: u64,
    selected_total_file_count: u64,
    groups: Vec<CleanupExportGroup>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanupExportGroup {
    category_id: String,
    category_name: String,
    description: String,
    risk_level: CleanupRisk,
    min_age_days: Option<u32>,
    size_bytes: u64,
    file_count: u64,
    paths: Vec<CleanupExportPath>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanupExportPath {
    path: String,
    size_bytes: u64,
    file_count: u64,
    matched_rule: String,
    source: String,
    scanned_at_ms: i64,
    volume_serial: Option<u64>,
    file_id: Option<u64>,
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
    pub scan_id: String,
    pub category_ids: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub confirm_caution: bool,
    pub confirm_advanced: bool,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LargeFileScanArgs {
    pub root: String,
    pub min_size_mb: u64,
    pub limit: u32,
}

#[derive(Debug, Default, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanCleanupArgs {
    pub project_roots: Vec<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn scan_cleanup(
    app: AppHandle,
    args: Option<ScanCleanupArgs>,
) -> Result<ScanResult, IpcError> {
    let _guard = CleanupTaskGuard::acquire(CleanupTaskKind::Scanning)?;
    clear_scan_cache();
    let _app_guard = ActiveScanApp::install(app.clone());
    SCAN_CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    let started = Instant::now();
    let started_at_ms = chrono::Local::now().timestamp_millis();
    let _ = app.emit(
        CLEANUP_SCAN_PROGRESS_EVENT,
        CleanupScanProgressEvent {
            stage: "queen".into(),
            phase: "准备扫描".into(),
            scanned_paths: 0,
            skipped_paths: 0,
            ignored_paths: 0,
            scout_tasks: 0,
            engineer_tasks: 0,
            current_path: None,
            done: false,
            cancelled: false,
        },
    );
    let project_roots = normalize_custom_project_roots(args.unwrap_or_default().project_roots);
    let exported_project_roots = project_roots
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let colony = ColonyConfig::detect();
    let mut result = tokio::task::spawn_blocking(move || do_scan(project_roots, colony))
        .await
        .map_err(|e| crate::error::AppError::Other(format!("cleanup scan join: {e}")))?;
    result.scan_id = format!(
        "{}-{}",
        chrono::Local::now().timestamp_millis(),
        NEXT_SCAN_ID.fetch_add(1, Ordering::Relaxed)
    );
    result.scanned_at_ms = started_at_ms;
    result.duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    result.project_roots = exported_project_roots;
    store_scan(&result);
    let _ = app.emit(
        CLEANUP_SCAN_PROGRESS_EVENT,
        CleanupScanProgressEvent {
            stage: "done".into(),
            phase: if result.cancelled {
                "扫描已取消".into()
            } else {
                "扫描完成".into()
            },
            scanned_paths: result.scanned_paths,
            skipped_paths: result.skipped_paths,
            ignored_paths: result.ignored_paths,
            scout_tasks: result.scout_tasks,
            engineer_tasks: result.engineer_tasks,
            current_path: None,
            done: true,
            cancelled: result.cancelled,
        },
    );
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_cleanup_scan() -> bool {
    if CLEANUP_STATE.load(Ordering::SeqCst) != CleanupTaskKind::Scanning as u8 {
        return false;
    }
    SCAN_CANCEL_REQUESTED.store(true, Ordering::SeqCst);
    true
}

#[tauri::command]
#[specta::specta]
pub async fn export_cleanup_scan(args: ExportCleanupArgs) -> Result<ExportCleanupResult, IpcError> {
    tokio::task::spawn_blocking(move || do_export_cleanup_scan(args))
        .await
        .map_err(|error| crate::error::AppError::Other(format!("cleanup export join: {error}")))?
}

fn do_export_cleanup_scan(args: ExportCleanupArgs) -> Result<ExportCleanupResult, IpcError> {
    let scan = cached_scan(&args.scan_id).ok_or_else(|| {
        crate::error::AppError::Invalid("扫描快照不存在，请重新扫描后再导出".into())
    })?;
    let document =
        build_cleanup_export_document(scan, args.selected_category_ids, args.excluded_paths);
    let path = PathBuf::from(args.path);
    let saved_to = write_json_atomic(&path, &document)?;
    Ok(ExportCleanupResult {
        saved_to,
        records: document.groups.iter().map(|group| group.paths.len()).sum(),
    })
}

fn build_cleanup_export_document(
    scan: ScanResult,
    selected_category_ids: Vec<String>,
    excluded_paths: Vec<String>,
) -> CleanupExportDocument {
    let selected_ids: std::collections::HashSet<String> =
        selected_category_ids.into_iter().collect();
    let excluded: std::collections::HashSet<String> = excluded_paths
        .into_iter()
        .map(|path| normalize_path_key(&path))
        .collect();
    let selected_categories = scan
        .categories
        .iter()
        .enumerate()
        .filter(|(_, category)| selected_ids.contains(&category.id))
        .collect::<Vec<_>>();
    let scanned_at_ms = scan.scanned_at_ms;
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(2, 8)
        .min(selected_categories.len().max(1));
    let next_category = AtomicUsize::new(0);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let selected_categories = &selected_categories;
            let excluded = &excluded;
            let next_category = &next_category;
            scope.spawn(move || loop {
                let job_index = next_category.fetch_add(1, Ordering::Relaxed);
                let Some((category_index, category)) = selected_categories.get(job_index) else {
                    break;
                };
                let paths = category
                    .paths
                    .iter()
                    .filter(|detail| !excluded.contains(&normalize_path_key(&detail.path)))
                    .map(|detail| CleanupExportPath {
                        path: detail.path.clone(),
                        size_bytes: detail.size_bytes,
                        file_count: detail.file_count,
                        matched_rule: detail.matched_rule.clone(),
                        source: detail.source.clone(),
                        scanned_at_ms,
                        volume_serial: detail.volume_serial,
                        file_id: detail.file_id,
                    })
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    continue;
                }
                let _ = sender.send((
                    *category_index,
                    CleanupExportGroup {
                        category_id: category.id.clone(),
                        category_name: category.name.clone(),
                        description: category.description.clone(),
                        risk_level: category.risk_level,
                        min_age_days: category.min_age_days,
                        size_bytes: paths.iter().map(|path| path.size_bytes).sum(),
                        file_count: paths.iter().map(|path| path.file_count).sum(),
                        paths,
                    },
                ));
            });
        }
    });
    drop(sender);

    let mut candidate_groups = receiver.into_iter().collect::<Vec<_>>();
    candidate_groups.sort_by_key(|(category_index, _)| *category_index);
    let mut exported_paths = std::collections::HashSet::new();
    let mut exported_identities = std::collections::HashSet::new();
    let mut groups = Vec::with_capacity(candidate_groups.len());
    for (_, mut group) in candidate_groups {
        group.paths.retain(|detail| {
            let path_key = normalize_path_key(&detail.path);
            let identity = detail.volume_serial.zip(detail.file_id);
            if exported_paths.contains(&path_key)
                || identity.is_some_and(|value| exported_identities.contains(&value))
            {
                return false;
            }
            exported_paths.insert(path_key);
            if let Some(identity) = identity {
                exported_identities.insert(identity);
            }
            true
        });
        if group.paths.is_empty() {
            continue;
        }
        group.size_bytes = group.paths.iter().map(|path| path.size_bytes).sum();
        group.file_count = group.paths.iter().map(|path| path.file_count).sum();
        groups.push(group);
    }
    let selected_total_size_bytes = groups.iter().map(|group| group.size_bytes).sum();
    let selected_total_file_count = groups.iter().map(|group| group.file_count).sum();
    CleanupExportDocument {
        schema_version: 3,
        scan_id: scan.scan_id,
        scanned_at_ms: scan.scanned_at_ms,
        duration_ms: scan.duration_ms,
        complete: scan.complete,
        cancelled: scan.cancelled,
        warnings: scan.warnings,
        project_roots: scan.project_roots,
        hotspot_count: scan.hotspot_count,
        scout_workers: scan.scout_workers,
        engineer_workers: scan.engineer_workers,
        scout_tasks: scan.scout_tasks,
        engineer_tasks: scan.engineer_tasks,
        selected_total_size_bytes,
        selected_total_file_count,
        groups,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn clean_categories(app: AppHandle, args: CleanArgs) -> Result<CleanResult, IpcError> {
    let _guard = CleanupTaskGuard::acquire(CleanupTaskKind::Cleaning)?;
    let ids = args.category_ids;
    let excluded = args.excluded_paths;
    let scan = cached_scan(&args.scan_id).ok_or_else(|| {
        crate::error::AppError::Invalid("扫描快照不存在，请重新扫描后再清理".into())
    })?;
    if !scan_is_cleanable(&scan) {
        return Err(crate::error::AppError::Invalid(
            "扫描已取消或结果不完整，请重新完整扫描后再清理".into(),
        )
        .into());
    }
    let excluded = normalize_excluded_paths(&scan, &excluded);
    let selected_categories: Vec<&CleanupCategory> = scan
        .categories
        .iter()
        .filter(|category| ids.iter().any(|id| id == &category.id))
        .collect();
    validate_risk_confirmation(
        selected_categories
            .iter()
            .map(|category| category.risk_level),
        args.confirm_caution,
        args.confirm_advanced,
    )?;
    let scan = take_cached_scan(&args.scan_id).ok_or_else(|| {
        crate::error::AppError::Invalid("扫描结果已失效，请重新扫描后再清理".into())
    })?;
    let result = tokio::task::spawn_blocking(move || do_clean(&app, scan, &ids, &excluded))
        .await
        .map_err(|e| crate::error::AppError::Other(format!("cleanup clean join: {e}")))?;
    Ok(result)
}

fn normalize_excluded_paths(scan: &ScanResult, excluded_paths: &[String]) -> Vec<String> {
    let known: std::collections::HashMap<String, String> = scan
        .categories
        .iter()
        .flat_map(|category| category.paths.iter())
        .map(|detail| (normalize_path_key(&detail.path), detail.path.clone()))
        .collect();
    let mut normalized = Vec::new();
    for excluded in excluded_paths {
        let key = normalize_path_key(excluded);
        let Some(canonical) = known.get(&key) else {
            continue;
        };
        if !normalized.contains(canonical) {
            normalized.push(canonical.clone());
        }
    }
    normalized
}

fn validate_risk_confirmation(
    risks: impl IntoIterator<Item = CleanupRisk>,
    confirm_caution: bool,
    confirm_advanced: bool,
) -> Result<(), IpcError> {
    let risks: Vec<_> = risks.into_iter().collect();
    if !confirm_advanced && risks.contains(&CleanupRisk::Advanced) {
        return Err(crate::error::AppError::Invalid("高级维护项未确认".into()).into());
    }
    if !confirm_caution && risks.contains(&CleanupRisk::Caution) {
        return Err(crate::error::AppError::Invalid("谨慎清理项未确认".into()).into());
    }
    Ok(())
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

#[cfg(test)]
#[path = "cleanup/tests.rs"]
mod tests;

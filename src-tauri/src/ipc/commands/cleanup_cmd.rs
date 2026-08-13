use crate::error::IpcError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

type FileIdentity = (u64, u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DirectoryMeasureKey {
    path: String,
    min_age_days: Option<u32>,
}

#[derive(Default)]
struct DirectoryMeasureCellState {
    result: Mutex<Option<(u64, u64)>>,
    ready: Condvar,
}

impl DirectoryMeasureCellState {
    fn complete(&self, result: (u64, u64)) {
        let mut value = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if value.is_none() {
            *value = Some(result);
            self.ready.notify_all();
        }
    }

    fn wait(&self) -> (u64, u64) {
        let mut value = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while value.is_none() {
            value = self
                .ready
                .wait(value)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        (*value).unwrap_or_default()
    }
}

type DirectoryMeasureCell = Arc<DirectoryMeasureCellState>;
type SharedDirectoryMeasures =
    Arc<Mutex<std::collections::HashMap<DirectoryMeasureKey, DirectoryMeasureCell>>>;

static CLEANUP_STATE: AtomicU8 = AtomicU8::new(CleanupTaskKind::Idle as u8);
static LAST_SCAN: OnceLock<Mutex<Option<CachedScan>>> = OnceLock::new();
static NEXT_SCAN_ID: AtomicU64 = AtomicU64::new(1);
static SCAN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
static ACTIVE_SCAN_DIAGNOSTICS: OnceLock<Mutex<Option<Arc<ScanDiagnostics>>>> = OnceLock::new();
static ACTIVE_SCAN_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
static ACTIVE_SCAN_DIRECTORY_MEASURES: OnceLock<Mutex<Option<SharedDirectoryMeasures>>> =
    OnceLock::new();
static ACTIVE_SCAN_FILE_QUEUE: OnceLock<Mutex<Option<Arc<FileScanQueue>>>> = OnceLock::new();
const CLEANUP_PROGRESS_EVENT: &str = "cleanup:progress";
const CLEANUP_SCAN_PROGRESS_EVENT: &str = "cleanup:scan-progress";
const SCAN_TTL: Duration = Duration::from_secs(10 * 60);
const DIRECTORY_SCAN_MAX_DEPTH: u32 = 64;
const MAX_SCAN_ENTRIES: u64 = 4_000_000;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(150);
const MAX_SCAN_WARNINGS: usize = 80;
const MAX_PROJECT_CACHE_PATHS: usize = 20_000;
const FIXED_VOLUME_PROBE_DEPTH: u32 = 3;
const MAX_FIXED_VOLUME_PROBE_TASKS: usize = 50_000;
const COMMAND_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const HOTSPOT_SCHEMA_VERSION: u32 = 1;
const HOTSPOT_MAX_ENTRIES: usize = 2_048;
const HOTSPOT_MAX_MISSES: u32 = 3;

#[derive(Clone, Copy)]
struct ColonyConfig {
    scout_workers: usize,
    engineer_workers: usize,
}

impl ColonyConfig {
    fn detect() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(4)
            .clamp(2, 16);
        Self {
            // 一个物理线程承载一个有界“蚁营”；目录任务是逻辑侦察蚁，可在营之间共享，
            // 因此扩大任务规模不等于无限创建系统线程。
            scout_workers: (parallelism / 2).clamp(2, 8),
            engineer_workers: ((parallelism * 2) / 3).clamp(2, 10),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HotspotEntry {
    path: String,
    category_id: String,
    matched_rule: String,
    last_seen_ms: i64,
    last_scanned_ms: i64,
    size_bytes: u64,
    file_count: u64,
    miss_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HotspotIndex {
    schema_version: u32,
    entries: Vec<HotspotEntry>,
}

impl Default for HotspotIndex {
    fn default() -> Self {
        Self {
            schema_version: HOTSPOT_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

struct FileScanJob {
    path: PathBuf,
    min_age_days: Option<u32>,
    measurement: DirectoryMeasureCell,
}

struct FileScanQueue {
    sender: std::sync::mpsc::Sender<FileScanJob>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl FileScanQueue {
    fn start(worker_count: usize) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel::<FileScanJob>();
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..worker_count.max(1))
            .filter_map(|index| {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("cleanup-engineer-{index}"))
                    .spawn(move || loop {
                        let job = receiver.lock().ok().and_then(|value| value.recv().ok());
                        let Some(job) = job else { break };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            dir_size_with_min_age_inline(&job.path, job.min_age_days)
                        }))
                        .unwrap_or_default();
                        job.measurement.complete(result);
                    })
                    .ok()
            })
            .collect();
        Self { sender, workers }
    }
}

impl Drop for FileScanQueue {
    fn drop(&mut self) {
        let (replacement, _) = std::sync::mpsc::channel();
        let sender = std::mem::replace(&mut self.sender, replacement);
        drop(sender);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

struct ActiveScanDiagnostics;

impl ActiveScanDiagnostics {
    fn install(diagnostics: Arc<ScanDiagnostics>) -> Self {
        if let Ok(mut active) = active_scan_diagnostics().lock() {
            *active = Some(diagnostics);
        }
        Self
    }
}

struct ActiveScanApp;

impl ActiveScanApp {
    fn install(app: AppHandle) -> Self {
        if let Ok(mut active) = active_scan_app().lock() {
            *active = Some(app);
        }
        Self
    }
}

impl Drop for ActiveScanApp {
    fn drop(&mut self) {
        if let Ok(mut active) = active_scan_app().lock() {
            *active = None;
        }
    }
}

impl Drop for ActiveScanDiagnostics {
    fn drop(&mut self) {
        if let Ok(mut active) = active_scan_diagnostics().lock() {
            *active = None;
        }
    }
}

struct ActiveScanDirectoryMeasures;

impl ActiveScanDirectoryMeasures {
    fn install(measures: SharedDirectoryMeasures) -> Self {
        if let Ok(mut active) = active_scan_directory_measures().lock() {
            *active = Some(measures);
        }
        Self
    }
}

struct ActiveScanFileQueue;

impl ActiveScanFileQueue {
    fn install(queue: Arc<FileScanQueue>) -> Self {
        if let Ok(mut active) = active_scan_file_queue().lock() {
            *active = Some(queue);
        }
        Self
    }
}

impl Drop for ActiveScanFileQueue {
    fn drop(&mut self) {
        if let Ok(mut active) = active_scan_file_queue().lock() {
            *active = None;
        }
    }
}

impl Drop for ActiveScanDirectoryMeasures {
    fn drop(&mut self) {
        if let Ok(mut active) = active_scan_directory_measures().lock() {
            *active = None;
        }
    }
}

fn active_scan_diagnostics() -> &'static Mutex<Option<Arc<ScanDiagnostics>>> {
    ACTIVE_SCAN_DIAGNOSTICS.get_or_init(|| Mutex::new(None))
}

fn active_scan_app() -> &'static Mutex<Option<AppHandle>> {
    ACTIVE_SCAN_APP.get_or_init(|| Mutex::new(None))
}

fn active_scan_directory_measures() -> &'static Mutex<Option<SharedDirectoryMeasures>> {
    ACTIVE_SCAN_DIRECTORY_MEASURES.get_or_init(|| Mutex::new(None))
}

fn active_scan_file_queue() -> &'static Mutex<Option<Arc<FileScanQueue>>> {
    ACTIVE_SCAN_FILE_QUEUE.get_or_init(|| Mutex::new(None))
}

fn current_scan_file_queue() -> Option<Arc<FileScanQueue>> {
    active_scan_file_queue()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

fn current_scan_directory_measures() -> Option<SharedDirectoryMeasures> {
    active_scan_directory_measures()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

fn with_scan_diagnostics(callback: impl FnOnce(&ScanDiagnostics)) {
    if let Ok(active) = active_scan_diagnostics().lock() {
        if let Some(diagnostics) = active.as_ref() {
            callback(diagnostics);
        }
    }
}

fn current_scan_diagnostics() -> Option<Arc<ScanDiagnostics>> {
    active_scan_diagnostics()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

#[derive(Clone)]
struct CachedScan {
    result: ScanResult,
    created_at: Instant,
}

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

fn scan_cache() -> &'static Mutex<Option<CachedScan>> {
    LAST_SCAN.get_or_init(|| Mutex::new(None))
}

fn cached_scan(scan_id: &str) -> Option<ScanResult> {
    scan_cache().lock().ok().and_then(|guard| {
        guard.as_ref().and_then(|cached| {
            (cached.result.scan_id == scan_id && cached.created_at.elapsed() <= SCAN_TTL)
                .then(|| cached.result.clone())
        })
    })
}

fn take_cached_scan(scan_id: &str) -> Option<ScanResult> {
    let mut guard = scan_cache().lock().ok()?;
    let matches = guard.as_ref().is_some_and(|cached| {
        cached.result.scan_id == scan_id && cached.created_at.elapsed() <= SCAN_TTL
    });
    matches.then(|| guard.take().expect("validated cached scan").result)
}

fn clear_scan_cache() {
    if let Ok(mut guard) = scan_cache().lock() {
        *guard = None;
    }
}

fn store_scan(scan: &ScanResult) {
    if let Ok(mut guard) = scan_cache().lock() {
        *guard = Some(CachedScan {
            result: scan.clone(),
            created_at: Instant::now(),
        });
    }
}

fn scan_is_cleanable(scan: &ScanResult) -> bool {
    scan.complete && !scan.cancelled && !scan.scan_id.is_empty()
}

fn load_hotspot_index() -> HotspotIndex {
    let path = crate::paths::cleanup_hotspots_file();
    let Ok(bytes) = std::fs::read(path) else {
        return HotspotIndex::default();
    };
    serde_json::from_slice::<HotspotIndex>(&bytes)
        .ok()
        .filter(|index| index.schema_version == HOTSPOT_SCHEMA_VERSION)
        .unwrap_or_default()
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<String, IpcError> {
    let parent = path
        .parent()
        .ok_or_else(|| crate::error::AppError::Invalid("导出路径没有父目录".into()))?;
    std::fs::create_dir_all(parent).map_err(crate::error::AppError::Io)?;
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ));
    let file = std::fs::File::create(&tmp).map_err(crate::error::AppError::Io)?;
    let mut writer = std::io::BufWriter::with_capacity(256 * 1024, file);
    serde_json::to_writer_pretty(&mut writer, value)
        .map_err(|error| crate::error::AppError::Other(format!("serialize json: {error}")))?;
    std::io::Write::flush(&mut writer).map_err(crate::error::AppError::Io)?;
    writer
        .get_ref()
        .sync_all()
        .map_err(crate::error::AppError::Io)?;
    drop(writer);
    replace_file(&tmp, path)?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<(), IpcError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(target_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| crate::error::AppError::Other(format!("replace json file: {error}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<(), IpcError> {
    std::fs::rename(source, target).map_err(crate::error::AppError::Io)?;
    Ok(())
}

fn save_hotspot_index(index: &HotspotIndex) {
    let _ = write_json_atomic(&crate::paths::cleanup_hotspots_file(), index);
}

fn hotspot_paths(index: &HotspotIndex) -> Vec<PathBuf> {
    let mut entries = index
        .entries
        .iter()
        .filter(|entry| entry.miss_count < HOTSPOT_MAX_MISSES)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_seen_ms));
    let mut paths = entries
        .into_iter()
        .map(|entry| PathBuf::from(&entry.path))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    deduplicate_paths(&mut paths);
    paths
}

fn update_hotspot_index(
    previous: HotspotIndex,
    categories: &[CleanupCategory],
    scanned_at_ms: i64,
) {
    let mut previous_by_key = previous
        .entries
        .into_iter()
        .map(|entry| (normalize_path_key(&entry.path), entry))
        .collect::<std::collections::HashMap<_, _>>();
    let mut entries = Vec::new();
    for category in categories.iter().filter(|category| {
        matches!(
            category.id.as_str(),
            "rust-target" | "cpp-cache" | "python-cache"
        )
    }) {
        for detail in &category.paths {
            let key = normalize_path_key(&detail.path);
            previous_by_key.remove(&key);
            entries.push(HotspotEntry {
                path: detail.path.clone(),
                category_id: category.id.clone(),
                matched_rule: detail.matched_rule.clone(),
                last_seen_ms: scanned_at_ms,
                last_scanned_ms: scanned_at_ms,
                size_bytes: detail.size_bytes,
                file_count: detail.file_count,
                miss_count: 0,
            });
        }
    }
    entries.extend(previous_by_key.into_values().filter_map(|mut entry| {
        entry.miss_count = entry.miss_count.saturating_add(1);
        entry.last_scanned_ms = scanned_at_ms;
        (entry.miss_count < HOTSPOT_MAX_MISSES && Path::new(&entry.path).is_dir()).then_some(entry)
    }));
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_seen_ms));
    entries.truncate(HOTSPOT_MAX_ENTRIES);
    save_hotspot_index(&HotspotIndex {
        schema_version: HOTSPOT_SCHEMA_VERSION,
        entries,
    });
}

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
    pub expires_at_ms: i64,
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
    result.expires_at_ms = chrono::Local::now().timestamp_millis() + SCAN_TTL.as_millis() as i64;
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
        crate::error::AppError::Invalid("扫描结果已过期，请重新扫描后再导出".into())
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
        crate::error::AppError::Invalid("扫描结果已过期，请重新扫描后再清理".into())
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

// ─── Scan logic ─────────────────────────────────────────────────────────────

struct ScanDiagnostics {
    scanned_paths: AtomicU64,
    skipped_paths: AtomicU64,
    ignored_paths: AtomicU64,
    scout_tasks: AtomicU64,
    engineer_tasks: AtomicU64,
    warnings: Mutex<Vec<String>>,
    started: Instant,
    timeout_reported: AtomicBool,
    budget_reported: AtomicBool,
    truncated: AtomicBool,
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
    fn warn(&self, message: impl Into<String>) {
        self.skipped_paths.fetch_add(1, Ordering::Relaxed);
        self.append_warning(message);
    }

    fn append_warning(&self, message: impl Into<String>) {
        if let Ok(mut warnings) = self.warnings.lock() {
            if warnings.len() < MAX_SCAN_WARNINGS {
                warnings.push(message.into());
            }
        }
    }

    fn skip_expected(&self, count: u64) {
        self.ignored_paths.fetch_add(count, Ordering::Relaxed);
    }

    fn dispatch_scout(&self) -> u64 {
        self.scout_tasks.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn dispatch_engineer(&self) -> u64 {
        self.engineer_tasks.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn try_visit(&self, current_path: &Path) -> bool {
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

    fn should_continue(&self, current_path: &Path) -> bool {
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

    fn snapshot(&self) -> (u64, u64, u64, Vec<String>) {
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
    fn progress(&self, stage: &str, phase: &str, current_path: Option<&Path>) {
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

fn do_scan(custom_project_roots: Vec<PathBuf>, colony: ColonyConfig) -> ScanResult {
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

    // 项目目录只遍历一次，同时识别 Rust、C/C++ 与 Python 缓存。
    diagnostics.progress("scout", "侦察蚁正在发现编程缓存", None);
    let project_caches = discover_project_caches(
        &custom_project_roots,
        &active_hotspot_paths,
        colony.scout_workers,
    );
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 5. Rust build cache (target dirs)
    if let Some(cat) = scan_rust_targets(&project_caches.rust) {
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
    if let Some(cat) = scan_python_cache(&project_caches.python) {
        categories.push(cat);
    }
    if scan_should_stop() {
        return finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    }

    // 6d. C/C++、.NET、Java 工具缓存与经过项目标记验证的构建产物
    if let Some(cat) = scan_cpp_cache(&project_caches.cpp) {
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

    let mut result = finalize_scan_result(categories, &diagnostics, colony, &hotspot_keys);
    update_hotspot_index(
        previous_hotspots,
        &result.categories,
        chrono::Local::now().timestamp_millis(),
    );
    result.hotspot_count = load_hotspot_index().entries.len() as u64;
    result
}

fn scan_should_stop() -> bool {
    SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed)
        || current_scan_diagnostics().is_some_and(|diagnostics| {
            diagnostics.started.elapsed() >= MAX_SCAN_DURATION
                || diagnostics.scanned_paths.load(Ordering::Relaxed) >= MAX_SCAN_ENTRIES
        })
}

fn finalize_scan_result(
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

fn deduplicate_category_paths(categories: &mut [CleanupCategory]) {
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
struct CleanupPolicy {
    risk: CleanupRisk,
    default_selected: bool,
    min_age_days: Option<u32>,
}

fn cleanup_policy(id: &str) -> CleanupPolicy {
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

fn category_matched_rule(id: &str) -> &'static str {
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

fn is_programming_category(id: &str) -> bool {
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

fn path_source(id: &str, path: &Path) -> &'static str {
    if matches!(id, "rust-target" | "cpp-cache" | "python-cache") && path.parent().is_some() {
        "project-discovery"
    } else if dirs::home_dir().is_some_and(|home| paths_overlap(&home, path)) {
        "user-directory"
    } else {
        "system-directory"
    }
}

fn programming_matched_rule(id: &str, path: &Path) -> &'static str {
    let normalized = normalize_path_key(&path.to_string_lossy());
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

fn programming_path_source(id: &str, path: &Path) -> &'static str {
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

fn queue_dir_size(
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

fn fixed_disk_roots() -> Vec<PathBuf> {
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

fn scan_rust_targets(discovered_targets: &[PathBuf]) -> Option<CleanupCategory> {
    let mut size = 0u64;
    let mut count = 0u64;
    let mut paths = Vec::new();

    let mut targets = discovered_targets.to_vec();
    if let Some(cargo_cache) =
        dirs::home_dir().map(|home| home.join(".cargo").join("registry").join("cache"))
    {
        push_cache_dir(&mut targets, cargo_cache);
    }
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let target_dir = PathBuf::from(target_dir);
        if is_rust_target_artifact_dir(&target_dir) {
            push_cache_dir(&mut targets, target_dir);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for config_path in [
            home.join(".cargo").join("config.toml"),
            home.join(".cargo").join("config"),
        ] {
            if let Some(target_dir) = cargo_target_dir_from_config(&config_path) {
                push_cache_dir(&mut targets, target_dir);
            }
        }
    }
    deduplicate_paths(&mut targets);

    for target_dir in &targets {
        let (s, c) = dir_size(target_dir);
        if s > 0 {
            size += s;
            count += c;
            paths.push(PathDetail {
                path: target_dir.to_string_lossy().to_string(),
                size_bytes: s,
                file_count: c,
                matched_rule: programming_matched_rule("rust-target", target_dir).to_string(),
                source: programming_path_source("rust-target", target_dir).to_string(),
                ..path_detail_identity(target_dir)
            });
        }
    }

    (size > 0).then(|| CleanupCategory {
        id: "rust-target".to_string(),
        name: "Rust 编译缓存".to_string(),
        description: "Cargo target 目录、显式 target-dir 和 registry 下载缓存".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("rust-target").risk,
        default_selected: cleanup_policy("rust-target").default_selected,
        min_age_days: cleanup_policy("rust-target").min_age_days,
    })
}

fn cargo_target_dir_from_config(config_path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config_path).ok()?;
    let value = toml::from_str::<toml::Value>(&text).ok()?;
    let target_dir = value.get("build")?.get("target-dir")?.as_str()?;
    let path = PathBuf::from(target_dir);
    let config_dir = config_path.parent()?;
    let base_dir = if config_dir
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(".cargo"))
    {
        config_dir.parent().unwrap_or(config_dir)
    } else {
        config_dir
    };
    let resolved = if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    };
    let resolved = safe_cleanup_root(&resolved).unwrap_or(resolved);
    is_rust_target_artifact_dir(&resolved).then_some(resolved)
}

fn cargo_config_target_from_project_root(project_root: &Path) -> Option<PathBuf> {
    let cargo_dir = project_root.join(".cargo");
    [cargo_dir.join("config.toml"), cargo_dir.join("config")]
        .into_iter()
        .find_map(|config| cargo_target_dir_from_config(&config))
}

fn scan_node_cache() -> Option<CleanupCategory> {
    let mut dirs_to_scan: Vec<PathBuf> = Vec::new();

    // npm cache（纯下载缓存，等同于 npm cache clean）
    if let Some(local) = dirs::data_local_dir() {
        let npm_cache = local.join("npm-cache");
        if npm_cache.exists() {
            dirs_to_scan.push(npm_cache);
        }
        // Bun 下载缓存；pnpm store 是项目硬链接依赖，不作为可安全清理项。
        push_cache_dir(
            &mut dirs_to_scan,
            dirs::home_dir()
                .unwrap_or_default()
                .join(".bun")
                .join("install")
                .join("cache"),
        );
        push_cache_dir(
            &mut dirs_to_scan,
            local.join("bun").join("install").join("cache"),
        );
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
                matched_rule: programming_matched_rule("node-cache", d).to_string(),
                source: programming_path_source("node-cache", d).to_string(),
                ..path_detail_identity(d)
            });
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "node-cache".to_string(),
        name: "Node.js 缓存".to_string(),
        description: "npm / Yarn / Bun 下载缓存（保留 pnpm 内容寻址存储）".to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("node-cache").risk,
        default_selected: cleanup_policy("node-cache").default_selected,
        min_age_days: cleanup_policy("node-cache").min_age_days,
    })
}

fn scan_python_cache(project_cache_dirs: &[PathBuf]) -> Option<CleanupCategory> {
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
                    matched_rule: programming_matched_rule("python-cache", &pip_cache).to_string(),
                    source: programming_path_source("python-cache", &pip_cache).to_string(),
                    ..path_detail_identity(&pip_cache)
                });
            }
        }
        for cache in [
            local.join("uv").join("cache"),
            local.join("pypoetry").join("Cache"),
            local.join("pdm").join("Cache"),
        ] {
            if cache.exists() {
                let (s, c) = dir_size(&cache);
                if s > 0 {
                    size += s;
                    count += c;
                    paths.push(PathDetail {
                        path: cache.to_string_lossy().to_string(),
                        size_bytes: s,
                        file_count: c,
                        matched_rule: programming_matched_rule("python-cache", &cache).to_string(),
                        source: programming_path_source("python-cache", &cache).to_string(),
                        ..path_detail_identity(&cache)
                    });
                }
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
                    matched_rule: programming_matched_rule("python-cache", &pipx).to_string(),
                    source: programming_path_source("python-cache", &pipx).to_string(),
                    ..path_detail_identity(&pipx)
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
                        matched_rule: programming_matched_rule("python-cache", &pkgs).to_string(),
                        source: programming_path_source("python-cache", &pkgs).to_string(),
                        ..path_detail_identity(&pkgs)
                    });
                }
                break;
            }
        }
    }

    for cache in project_cache_dirs {
        let (s, c) = dir_size(cache);
        if s > 0 {
            size += s;
            count += c;
            paths.push(PathDetail {
                path: cache.to_string_lossy().to_string(),
                size_bytes: s,
                file_count: c,
                matched_rule: programming_matched_rule("python-cache", cache).to_string(),
                source: programming_path_source("python-cache", cache).to_string(),
                ..path_detail_identity(cache)
            });
        }
    }

    if size == 0 {
        return None;
    }

    Some(CleanupCategory {
        id: "python-cache".to_string(),
        name: "Python 缓存".to_string(),
        description: "pip / pipx / uv / Poetry / PDM / conda 下载缓存及已验证项目工具缓存"
            .to_string(),
        size_bytes: size,
        file_count: count,
        paths,
        risk_level: cleanup_policy("python-cache").risk,
        default_selected: cleanup_policy("python-cache").default_selected,
        min_age_days: cleanup_policy("python-cache").min_age_days,
    })
}

fn scan_go_cache() -> Option<CleanupCategory> {
    // 只清 GOCACHE（编译缓存），不清 GOMODCACHE（模块源码缓存）。
    let go_cache = command_output_path("go", &["env", "GOCACHE"])
        .or_else(|| dirs::data_local_dir().map(|p| p.join("go-build")));

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
                    matched_rule: programming_matched_rule("go-cache", d).to_string(),
                    source: programming_path_source("go-cache", d).to_string(),
                    ..path_detail_identity(d)
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
        risk_level: cleanup_policy("go-cache").risk,
        default_selected: cleanup_policy("go-cache").default_selected,
        min_age_days: cleanup_policy("go-cache").min_age_days,
    })
}

fn command_output_path(command: &str, args: &[&str]) -> Option<PathBuf> {
    command_output_path_in(command, args, None)
}

fn command_output_path_in(
    command: &str,
    args: &[&str],
    current_dir: Option<&Path>,
) -> Option<PathBuf> {
    let mut command = std::process::Command::new(command);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    loop {
        if SCAN_CANCEL_REQUESTED.load(Ordering::Relaxed)
            || started.elapsed() >= COMMAND_DISCOVERY_TIMEOUT
        {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        match child.try_wait().ok()? {
            Some(_) => break,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().trim_matches('"');
    (!value.is_empty() && value != "off").then(|| PathBuf::from(value))
}

fn scan_cpp_cache(discovered_build_dirs: &[PathBuf]) -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        for path in [
            local.join("ccache"),
            local.join("sccache"),
            local.join("vcpkg").join("archives"),
        ] {
            push_cache_dir(&mut cache_dirs, path);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for path in [home.join(".ccache"), home.join(".cache").join("sccache")] {
            push_cache_dir(&mut cache_dirs, path);
        }
    }
    for path in discovered_build_dirs {
        push_cache_dir(&mut cache_dirs, path.clone());
    }
    deduplicate_paths(&mut cache_dirs);
    scan_existing_dirs_category(
        "cpp-cache",
        "C/C++ 编译缓存",
        "已验证的 CMake/MSBuild 构建产物及 ccache/sccache/vcpkg 下载缓存（保留 Conan 包仓库）",
        cache_dirs,
    )
}

fn scan_dotnet_cache() -> Option<CleanupCategory> {
    let mut cache_dirs = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        push_cache_dir(&mut cache_dirs, local.join("NuGet").join("v3-cache"));
        push_cache_dir(&mut cache_dirs, local.join("NuGet").join("plugins-cache"));
        push_cache_dir(&mut cache_dirs, local.join("Temp").join("NuGetScratch"));
    }
    scan_existing_dirs_category(
        "dotnet-cache",
        ".NET/NuGet 缓存",
        "NuGet HTTP、插件与临时缓存（保留全局 packages 依赖目录）",
        cache_dirs,
    )
}

fn scan_java_cache() -> Option<CleanupCategory> {
    let home = dirs::home_dir()?;
    let mut cache_dirs = Vec::new();
    for path in [
        home.join(".gradle").join("caches"),
        home.join(".gradle").join("wrapper").join("dists"),
        home.join(".m2").join("repository").join(".cache"),
    ] {
        push_cache_dir(&mut cache_dirs, path);
    }
    scan_existing_dirs_category(
        "java-cache",
        "Java 构建缓存",
        "Gradle 缓存与 wrapper 下载包（保留 Maven 本地依赖仓库）",
        cache_dirs,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectScoutMode {
    Deep,
    FixedVolumeProbe,
}

#[derive(Clone)]
struct ProjectSearchRoot {
    path: PathBuf,
    mode: ProjectScoutMode,
}

fn push_project_search_root(
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

fn project_search_plan(
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
fn project_search_roots(custom_roots: &[PathBuf]) -> Vec<PathBuf> {
    let fixed_roots = fixed_disk_roots();
    project_search_plan(custom_roots, &fixed_roots)
        .into_iter()
        .map(|root| root.path)
        .collect()
}

#[derive(Default)]
struct ProjectCacheDiscovery {
    rust: Vec<PathBuf>,
    cpp: Vec<PathBuf>,
    python: Vec<PathBuf>,
}

#[derive(Clone)]
struct ProjectScoutTask {
    root: PathBuf,
    dir: PathBuf,
    depth: u32,
    mode: ProjectScoutMode,
}

struct ProjectScoutQueue {
    tasks: Mutex<std::collections::VecDeque<ProjectScoutTask>>,
    ready: Condvar,
    pending: AtomicUsize,
}

impl ProjectScoutQueue {
    #[cfg(test)]
    fn new(roots: &[PathBuf]) -> Self {
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

    fn from_plan(roots: &[ProjectSearchRoot]) -> Self {
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

    fn push(&self, task: ProjectScoutTask) {
        self.pending.fetch_add(1, Ordering::Relaxed);
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.push_back(task);
        self.ready.notify_one();
    }

    fn pop(&self) -> Option<ProjectScoutTask> {
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

    fn complete(&self) {
        let _tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.ready.notify_all();
        }
    }
}

fn discover_project_caches(
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

fn normalize_custom_project_roots(roots: Vec<String>) -> Vec<PathBuf> {
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

fn is_safe_project_root(path: &Path) -> bool {
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

fn scan_project_cache_task(
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

fn is_project_container_name(name: &str) -> bool {
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

fn directory_has_project_marker(path: &Path) -> bool {
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

fn is_separately_assigned_root(root: &Path, path: &Path, assigned_roots: &[PathBuf]) -> bool {
    assigned_roots
        .iter()
        .any(|assigned| !same_path(assigned, root) && same_path(assigned, path))
}

fn project_search_depth(root: &Path) -> u32 {
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

fn should_skip_project_path(root: &Path, path: &Path, name: &str, depth: u32) -> bool {
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

fn dedicated_user_roots() -> Vec<PathBuf> {
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

fn dedicated_volume_roots(root: &Path) -> Vec<PathBuf> {
    ["Code", "code", "Dev", "dev", "Projects", "projects", "src"]
        .into_iter()
        .map(|name| root.join(name))
        .filter(|path| path.exists())
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    if normalize_path_key(&left.to_string_lossy()) == normalize_path_key(&right.to_string_lossy()) {
        return true;
    }
    match (directory_identity(left), directory_identity(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn is_volume_root(path: &Path) -> bool {
    path.parent().is_none() || path.parent().is_some_and(|parent| parent == path)
}

fn is_verified_rust_target(path: &Path, name: &str) -> bool {
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

fn is_rust_target_artifact_dir(path: &Path) -> bool {
    path.join("CACHEDIR.TAG").is_file()
        || path.join(".rustc_info.json").is_file()
        || path.join("debug").is_dir()
        || path.join("release").is_dir()
}

fn is_verified_cpp_build_dir(path: &Path, name: &str) -> bool {
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

fn directory_has_extension(path: &Path, extensions: &[&str]) -> bool {
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

fn is_verified_python_project_cache(path: &Path, name: &str) -> bool {
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

fn deduplicate_directory_roots(paths: &mut Vec<PathBuf>) {
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

fn deduplicate_paths(paths: &mut Vec<PathBuf>) {
    deduplicate_directory_roots(paths);
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
                                ..path_detail_identity(&cache2)
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
                    ..path_detail_identity(cache_dir)
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
        risk_level: cleanup_policy("browser-cache").risk,
        default_selected: cleanup_policy("browser-cache").default_selected,
        min_age_days: cleanup_policy("browser-cache").min_age_days,
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
        for name in ["Cache", "Code Cache", "GPUCache", "Crashpad"].iter() {
            let path = if *name == "Crashpad" {
                root.join(name).join("reports")
            } else {
                root.join(name)
            };
            push_cache_dir(&mut cache_dirs, path);
        }
    }

    if let Some(roaming) = roaming {
        for app in [
            "Code",
            "Cursor",
            "VSCodium",
            "Slack",
            "Discord",
            "discordcanary",
            "discordptb",
        ] {
            let root = roaming.join(app);
            for name in ["Cache", "Code Cache", "GPUCache"].iter() {
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
    failed_items: u64,
    last_emit: Instant,
}

#[derive(Clone, Copy)]
struct CleanupFilePolicy {
    min_age_days: Option<u32>,
    scanned_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupPolicyDecision {
    Allow,
    NotEligibleAtScan,
    ChangedAfterScan,
    MetadataUnavailable,
}

#[derive(Default)]
struct PathCleanupDiagnostics {
    changed_entries: u64,
    unverifiable_entries: u64,
    examples: Vec<String>,
}

impl PathCleanupDiagnostics {
    fn record(&mut self, path: &Path, decision: CleanupPolicyDecision) {
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

    fn has_safety_skips(&self) -> bool {
        self.changed_entries > 0 || self.unverifiable_entries > 0
    }
}

impl CleanupFilePolicy {
    fn for_category(category: &CleanupCategory, scan: &ScanResult) -> Self {
        Self {
            min_age_days: category.min_age_days,
            scanned_at: scan_timestamp(scan.scanned_at_ms),
        }
    }

    fn file_decision(&self, metadata: &std::fs::Metadata) -> CleanupPolicyDecision {
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

    fn directory_decision(&self, metadata: &std::fs::Metadata) -> CleanupPolicyDecision {
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

fn scan_timestamp(scanned_at_ms: i64) -> Option<SystemTime> {
    (scanned_at_ms > 0)
        .then(|| SystemTime::UNIX_EPOCH + Duration::from_millis(scanned_at_ms as u64))
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
            failed_items: 0,
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

    fn fail(&mut self, processed: u64) {
        self.failed_items = self.failed_items.saturating_add(processed);
        self.skip(processed);
    }

    fn finish(&mut self) {
        self.emit(true, true);
    }

    fn emit(&mut self, force: bool, done: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_millis(120) {
            return;
        }
        let percent = self
            .processed_items
            .saturating_mul(100)
            .checked_div(self.total_items)
            .map(|value| value.min(if done { 100 } else { 99 }) as u8)
            .unwrap_or(if done { 100 } else { 0 });
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

fn remaining_path_items(processed_before: u64, processed_after: u64, expected_items: u64) -> u64 {
    let processed_for_path = processed_after
        .saturating_sub(processed_before)
        .min(expected_items);
    expected_items.saturating_sub(processed_for_path)
}

fn settle_path_progress(
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

fn append_path_cleanup_diagnostics(
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

fn do_clean(
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
        match cat.id.as_str() {
            // 缩略图：只删 thumbcache_*.db / iconcache_*.db，不能整目录清空
            "thumbnails" => {
                for detail in &cat.paths {
                    if excluded_set.contains(detail.path.as_str()) {
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
                    match remove_thumbnail_files_with_progress(
                        &path,
                        &mut progress,
                        &mut errors,
                        file_policy,
                        &mut diagnostics,
                    ) {
                        Ok((_s, _c)) => {
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                false,
                                &mut errors,
                            );
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                true,
                                &mut errors,
                            );
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
                    let blockers = running_process_blockers_for_path(
                        &cat.id,
                        &detail.path,
                        &running_processes,
                    );
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
                    let is_conda_pkgs = path.ends_with("pkgs")
                        && path.to_string_lossy().to_lowercase().contains("conda");
                    let result = if is_conda_pkgs {
                        remove_conda_archives_with_progress(
                            &path,
                            &mut progress,
                            &mut errors,
                            file_policy,
                            &mut diagnostics,
                        )
                    } else {
                        remove_dir_contents_with_progress(
                            &path,
                            &mut progress,
                            file_policy,
                            &mut errors,
                            &mut diagnostics,
                        )
                    };
                    match result {
                        Ok((_s, _c)) => {
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                false,
                                &mut errors,
                            );
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                true,
                                &mut errors,
                            );
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
                    let blockers = running_process_blockers_for_path(
                        &cat.id,
                        &detail.path,
                        &running_processes,
                    );
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
                    match remove_dir_contents_with_progress(
                        &path,
                        &mut progress,
                        file_policy,
                        &mut errors,
                        &mut diagnostics,
                    ) {
                        Ok((_s, _c)) => {
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                false,
                                &mut errors,
                            );
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", detail.path, e));
                            let remaining = settle_path_progress(
                                &mut progress,
                                processed_before,
                                expected_items,
                            );
                            append_path_cleanup_diagnostics(
                                &detail.path,
                                &diagnostics,
                                remaining,
                                true,
                                &mut errors,
                            );
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

#[derive(Debug, Clone)]
struct RunningProcessInfo {
    name: String,
    stem: String,
    command_paths: Vec<String>,
    cwd: Option<String>,
}

fn running_process_snapshot() -> Vec<RunningProcessInfo> {
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

fn process_argument_path_keys(argument: &str) -> Vec<String> {
    let mut values = vec![normalize_path_key(argument)];
    if let Some((_, value)) = argument.split_once('=') {
        let normalized = normalize_path_key(value);
        if !normalized.is_empty() && !values.contains(&normalized) {
            values.push(normalized);
        }
    }
    values
}

fn watched_processes(category_id: &str) -> Option<(&'static [&'static str], bool)> {
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
    let block_anywhere = matches!(
        category_id,
        "rust-target"
            | "node-cache"
            | "python-cache"
            | "go-cache"
            | "cpp-cache"
            | "dotnet-cache"
            | "java-cache"
    );
    Some((watched, block_anywhere))
}

fn running_process_blockers_for_path(
    category_id: &str,
    path: &str,
    processes: &[RunningProcessInfo],
) -> Vec<String> {
    let Some((watched, block_anywhere)) = watched_processes(category_id) else {
        return Vec::new();
    };
    let normalized_path = normalize_path_key(path);
    let mut found = std::collections::BTreeSet::new();
    for process in processes {
        if !watched.contains(&process.stem.as_str()) {
            continue;
        }
        let uses_path = process
            .command_paths
            .iter()
            .any(|part| part.len() > 3 && paths_overlap_keys(part, &normalized_path))
            || process
                .cwd
                .as_ref()
                .is_some_and(|cwd| paths_overlap_keys(cwd, &normalized_path));
        if block_anywhere || uses_path {
            found.insert(process.name.clone());
        }
    }
    found.into_iter().collect()
}

fn paths_overlap_keys(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('\\'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn normalize_path_key(value: &str) -> String {
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

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = normalize_path_key(&left.to_string_lossy());
    let right = normalize_path_key(&right.to_string_lossy());
    paths_overlap_keys(&left, &right)
}

fn remove_dir_contents_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    file_policy: CleanupFilePolicy,
    errors: &mut Vec<String>,
    diagnostics: &mut PathCleanupDiagnostics,
) -> std::io::Result<(u64, u64)> {
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
            Ok(m) => m,
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
            let (s, c) = remove_dir_tree_with_progress(
                &path,
                progress,
                file_policy,
                1,
                DIRECTORY_SCAN_MAX_DEPTH,
                errors,
                diagnostics,
            )?;
            freed += s;
            count += c;
        } else {
            let decision = file_policy.file_decision(&meta);
            if decision != CleanupPolicyDecision::Allow {
                diagnostics.record(&path, decision);
                continue;
            }
            let size = file_details(&path, &meta)
                .map(|value| value.2)
                .unwrap_or_else(|| meta.len());
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
                count += 1;
                progress.add_result(1, size, 1);
            } else {
                errors.push(format!("{}: 删除文件失败", path.display()));
                progress.fail(1);
            }
        }
    }

    Ok((freed, count))
}

fn remove_dir_tree_with_progress(
    dir: &Path,
    progress: &mut CleanProgress<'_>,
    file_policy: CleanupFilePolicy,
    depth: u32,
    max_depth: u32,
    errors: &mut Vec<String>,
    diagnostics: &mut PathCleanupDiagnostics,
) -> std::io::Result<(u64, u64)> {
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
            let (s, c) = remove_dir_tree_with_progress(
                &path,
                progress,
                file_policy,
                depth + 1,
                max_depth,
                errors,
                diagnostics,
            )?;
            freed += s;
            count += c;
        } else {
            let decision = file_policy.file_decision(&meta);
            if decision != CleanupPolicyDecision::Allow {
                diagnostics.record(&path, decision);
                continue;
            }
            let size = file_details(&path, &meta)
                .map(|value| value.2)
                .unwrap_or_else(|| meta.len());
            if std::fs::remove_file(&path).is_ok() {
                freed += size;
                count += 1;
                progress.add_result(1, size, 1);
            } else {
                errors.push(format!("{}: 删除文件失败", path.display()));
                progress.fail(1);
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
    files.sort_by_key(|file| std::cmp::Reverse(file.size_bytes));
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
    results.sort_by_key(|file| std::cmp::Reverse(file.size_bytes));
    results.truncate(limit);
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn dir_size(path: &Path) -> (u64, u64) {
    dir_size_with_min_age(path, None)
}

fn dir_size_with_min_age(path: &Path, min_age_days: Option<u32>) -> (u64, u64) {
    schedule_dir_size(path, min_age_days).wait()
}

fn schedule_dir_size(path: &Path, min_age_days: Option<u32>) -> DirectoryMeasureCell {
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

fn schedule_dir_size_uncached(path: &Path, min_age_days: Option<u32>) -> DirectoryMeasureCell {
    let measurement = Arc::new(DirectoryMeasureCellState::default());
    if !queue_dir_size(path, min_age_days, measurement.clone()) {
        measurement.complete(dir_size_with_min_age_inline(path, min_age_days));
    }
    measurement
}

fn dir_size_with_min_age_inline(path: &Path, min_age_days: Option<u32>) -> (u64, u64) {
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
struct DirectoryMeasure {
    size: u64,
    count: u64,
    warnings: Vec<String>,
    stopped: bool,
}

fn dir_size_recursive(
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

fn push_measure_warning(result: &mut DirectoryMeasure, warning: String) {
    if result.warnings.len() < MAX_SCAN_WARNINGS {
        result.warnings.push(warning);
    }
}

fn safe_cleanup_root(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() || metadata_is_reparse_point(&meta) {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

fn path_detail_identity(path: &Path) -> PathDetail {
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

fn revalidate_cleanup_root(expected: &PathDetail) -> Option<PathBuf> {
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
fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    file_handle_identity(path, true).map(|value| (value.0, value.1))
}

#[cfg(windows)]
fn file_details(path: &Path, _metadata: &std::fs::Metadata) -> Option<(u64, u64, u64)> {
    file_handle_identity(path, false)
}

#[cfg(windows)]
fn file_handle_identity(path: &Path, directory: bool) -> Option<(u64, u64, u64)> {
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
fn directory_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(windows))]
fn file_details(_path: &Path, metadata: &std::fs::Metadata) -> Option<(u64, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((
        metadata.dev(),
        metadata.ino(),
        metadata.blocks().saturating_mul(512),
    ))
}

fn metadata_is_reparse_point(meta: &std::fs::Metadata) -> bool {
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
            matched_rule: category_matched_rule("thumbnails").to_string(),
            source: path_source("thumbnails", dir).to_string(),
            ..path_detail_identity(dir)
        }],
        risk_level: cleanup_policy("thumbnails").risk,
        default_selected: cleanup_policy("thumbnails").default_selected,
        min_age_days: cleanup_policy("thumbnails").min_age_days,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File, FileTimes};
    use std::io::Write;

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "syspulse-cleanup-test-{name}-{}-{}",
            std::process::id(),
            NEXT_SCAN_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn only_safe_categories_are_selected_by_default() {
        assert!(cleanup_policy("win-temp").default_selected);
        assert!(cleanup_policy("installer-cache").default_selected);
        for id in [
            "browser-cache",
            "app-cache",
            "webview-cache",
            "rust-target",
            "shader-cache",
        ] {
            assert!(!cleanup_policy(id).default_selected, "{id}");
        }
    }

    #[test]
    fn age_filter_excludes_recent_files() {
        let root = temp_root("age");
        fs::create_dir_all(&root).unwrap();
        let recent = root.join("recent.tmp");
        let old = root.join("old.tmp");
        fs::write(&recent, vec![1u8; 11]).unwrap();
        let mut file = File::create(&old).unwrap();
        file.write_all(&[2u8; 17]).unwrap();
        file.set_times(
            FileTimes::new()
                .set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60)),
        )
        .unwrap();

        let expected = file_details(&old, &fs::metadata(&old).unwrap())
            .map(|value| value.2)
            .unwrap_or(17);
        assert_eq!(dir_size_with_min_age(&root, Some(7)), (expected, 1));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_root_must_be_a_real_directory() {
        let root = temp_root("root");
        fs::write(&root, b"not a directory").unwrap();
        assert!(safe_cleanup_root(&root).is_none());
        fs::remove_file(root).unwrap();
    }

    #[test]
    fn caution_category_requires_explicit_confirmation() {
        assert!(validate_risk_confirmation([CleanupRisk::Caution], false, false).is_err());
        assert!(validate_risk_confirmation([CleanupRisk::Caution], true, false).is_ok());
    }

    #[test]
    fn advanced_category_requires_explicit_confirmation() {
        assert!(validate_risk_confirmation([CleanupRisk::Advanced], true, false).is_err());
        assert!(validate_risk_confirmation([CleanupRisk::Advanced], false, true).is_ok());
    }

    #[test]
    fn cancelled_or_incomplete_scan_is_never_cleanable() {
        let base = ScanResult {
            scan_id: "scan-1".into(),
            categories: Vec::new(),
            total_size_bytes: 0,
            total_file_count: 0,
            scanned_at_ms: 0,
            expires_at_ms: 0,
            duration_ms: 0,
            scanned_paths: 0,
            skipped_paths: 0,
            ignored_paths: 0,
            hotspot_count: 0,
            scout_workers: 1,
            engineer_workers: 2,
            scout_tasks: 0,
            engineer_tasks: 0,
            project_roots: Vec::new(),
            warnings: Vec::new(),
            complete: true,
            cancelled: false,
        };
        assert!(scan_is_cleanable(&base));
        assert!(!scan_is_cleanable(&ScanResult {
            cancelled: true,
            ..base.clone()
        }));
        assert!(!scan_is_cleanable(&ScanResult {
            complete: false,
            ..base.clone()
        }));
        assert!(!scan_is_cleanable(&ScanResult {
            scan_id: String::new(),
            ..base
        }));
    }

    #[test]
    fn path_normalization_handles_extended_windows_prefixes() {
        assert_eq!(normalize_path_key(r"\\?\C:\Temp\"), r"c:\temp");
        assert_eq!(
            normalize_path_key(r"\\?\UNC\server\share\Cache"),
            r"\\server\share\cache"
        );
        assert!(paths_overlap_keys(r"c:\work", r"c:\work\build"));
        assert!(!paths_overlap_keys(r"c:\work", r"c:\workspace"));
    }

    #[test]
    fn scan_result_contains_only_supplied_categories() {
        let diagnostics = ScanDiagnostics::default();
        let categories = vec![CleanupCategory {
            id: "win-temp".into(),
            name: "Windows 临时文件".into(),
            description: "test".into(),
            size_bytes: 0,
            file_count: 0,
            paths: Vec::new(),
            risk_level: CleanupRisk::Safe,
            default_selected: true,
            min_age_days: Some(7),
        }];
        let result = finalize_scan_result(
            categories,
            &diagnostics,
            ColonyConfig {
                scout_workers: 1,
                engineer_workers: 2,
            },
            &std::collections::HashSet::new(),
        );
        assert_eq!(result.categories.len(), 1);
        assert_eq!(result.categories[0].id, "win-temp");
    }

    #[test]
    fn hotspot_paths_prioritize_recent_entries_and_prune_missing_paths() {
        let root = temp_root("hotspot-order");
        let older = root.join("older");
        let newer = root.join("newer");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&newer).unwrap();
        let index = HotspotIndex {
            schema_version: HOTSPOT_SCHEMA_VERSION,
            entries: vec![
                HotspotEntry {
                    path: older.to_string_lossy().to_string(),
                    category_id: "rust-target".into(),
                    matched_rule: "old".into(),
                    last_seen_ms: 1,
                    last_scanned_ms: 1,
                    size_bytes: 1,
                    file_count: 1,
                    miss_count: 0,
                },
                HotspotEntry {
                    path: newer.to_string_lossy().to_string(),
                    category_id: "cpp-cache".into(),
                    matched_rule: "new".into(),
                    last_seen_ms: 2,
                    last_scanned_ms: 2,
                    size_bytes: 2,
                    file_count: 2,
                    miss_count: 0,
                },
                HotspotEntry {
                    path: root.join("missing").to_string_lossy().to_string(),
                    category_id: "python-cache".into(),
                    matched_rule: "missing".into(),
                    last_seen_ms: 3,
                    last_scanned_ms: 3,
                    size_bytes: 3,
                    file_count: 3,
                    miss_count: 0,
                },
            ],
        };
        let paths = hotspot_paths(&index);
        assert_eq!(paths, vec![newer, older]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hotspot_index_round_trips_and_corrupt_json_falls_back_to_default() {
        let root = temp_root("hotspot-json");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("hotspots.json");
        let index = HotspotIndex {
            schema_version: HOTSPOT_SCHEMA_VERSION,
            entries: vec![HotspotEntry {
                path: root.to_string_lossy().to_string(),
                category_id: "rust-target".into(),
                matched_rule: "Cargo.toml + target".into(),
                last_seen_ms: 10,
                last_scanned_ms: 11,
                size_bytes: 12,
                file_count: 13,
                miss_count: 0,
            }],
        };
        write_json_atomic(&path, &index).unwrap();
        let decoded: HotspotIndex = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded.entries.len(), 1);
        fs::write(&path, b"not-json").unwrap();
        let decoded = serde_json::from_slice::<HotspotIndex>(&fs::read(&path).unwrap())
            .ok()
            .filter(|value| value.schema_version == HOTSPOT_SCHEMA_VERSION)
            .unwrap_or_default();
        assert!(decoded.entries.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_document_records_category_and_path_selection() {
        let selected_path = PathDetail {
            path: r"C:\cache\selected".into(),
            size_bytes: 10,
            file_count: 1,
            matched_rule: "rule".into(),
            source: "tool-global-cache".into(),
            volume_serial: Some(1),
            file_id: Some(1),
        };
        let excluded_path = PathDetail {
            path: r"C:\cache\excluded".into(),
            size_bytes: 20,
            file_count: 2,
            matched_rule: "rule".into(),
            source: "tool-global-cache".into(),
            volume_serial: Some(1),
            file_id: Some(2),
        };
        let scan = ScanResult {
            scan_id: "scan-export".into(),
            categories: vec![CleanupCategory {
                id: "node-cache".into(),
                name: "Node.js 缓存".into(),
                description: "test".into(),
                size_bytes: 30,
                file_count: 3,
                paths: vec![selected_path.clone(), excluded_path.clone()],
                risk_level: CleanupRisk::Advanced,
                default_selected: false,
                min_age_days: None,
            }],
            total_size_bytes: 30,
            total_file_count: 3,
            scanned_at_ms: 100,
            expires_at_ms: 200,
            duration_ms: 5,
            scanned_paths: 2,
            skipped_paths: 0,
            ignored_paths: 0,
            hotspot_count: 1,
            scout_workers: 1,
            engineer_workers: 2,
            scout_tasks: 2,
            engineer_tasks: 2,
            project_roots: vec![r"C:\work".into()],
            warnings: Vec::new(),
            complete: true,
            cancelled: false,
        };
        let document = build_cleanup_export_document(
            scan,
            vec!["node-cache".into()],
            vec![excluded_path.path],
        );
        assert_eq!(document.groups.len(), 1);
        assert_eq!(document.groups[0].paths.len(), 1);
        assert_eq!(document.groups[0].paths[0].path, selected_path.path);
        assert_eq!(document.project_roots, vec![r"C:\work"]);
        assert_eq!(document.schema_version, 3);
        assert_eq!(document.selected_total_size_bytes, 10);
        assert_eq!(document.selected_total_file_count, 1);
        assert_eq!(document.groups[0].paths[0].volume_serial, Some(1));
        assert_eq!(document.groups[0].paths[0].file_id, Some(1));
    }

    #[test]
    fn export_document_omits_unselected_categories() {
        let scan = ScanResult {
            scan_id: "scan-export-filter".into(),
            categories: vec![CleanupCategory {
                id: "rust-target".into(),
                name: "Rust 编译缓存".into(),
                description: "test".into(),
                size_bytes: 42,
                file_count: 2,
                paths: vec![PathDetail {
                    path: r"C:\work\target".into(),
                    size_bytes: 42,
                    file_count: 2,
                    matched_rule: "Cargo target".into(),
                    source: "project-discovery".into(),
                    volume_serial: Some(1),
                    file_id: Some(2),
                }],
                risk_level: CleanupRisk::Advanced,
                default_selected: false,
                min_age_days: None,
            }],
            total_size_bytes: 42,
            total_file_count: 2,
            scanned_at_ms: 100,
            expires_at_ms: 200,
            duration_ms: 5,
            scanned_paths: 2,
            skipped_paths: 0,
            ignored_paths: 0,
            hotspot_count: 0,
            scout_workers: 2,
            engineer_workers: 2,
            scout_tasks: 12,
            engineer_tasks: 1,
            project_roots: Vec::new(),
            warnings: Vec::new(),
            complete: true,
            cancelled: false,
        };
        let document = build_cleanup_export_document(scan, Vec::new(), Vec::new());
        assert!(document.groups.is_empty());
        assert_eq!(document.selected_total_size_bytes, 0);
    }

    #[test]
    fn duplicate_paths_are_owned_by_the_first_category_once() {
        let shared = PathDetail {
            path: r"C:\cache\shared".into(),
            size_bytes: 10,
            file_count: 2,
            matched_rule: "shared".into(),
            source: "tool-global-cache".into(),
            volume_serial: Some(7),
            file_id: Some(9),
        };
        let mut categories = vec![
            CleanupCategory {
                id: "node-cache".into(),
                name: "Node".into(),
                description: String::new(),
                size_bytes: 10,
                file_count: 2,
                paths: vec![shared.clone()],
                risk_level: CleanupRisk::Advanced,
                default_selected: false,
                min_age_days: None,
            },
            CleanupCategory {
                id: "app-cache".into(),
                name: "App".into(),
                description: String::new(),
                size_bytes: 10,
                file_count: 2,
                paths: vec![PathDetail {
                    path: r"\\?\C:\CACHE\SHARED".into(),
                    ..shared
                }],
                risk_level: CleanupRisk::Caution,
                default_selected: false,
                min_age_days: None,
            },
        ];

        deduplicate_category_paths(&mut categories);

        assert_eq!(categories[0].paths.len(), 1);
        assert!(categories[1].paths.is_empty());
        assert_eq!(categories[1].size_bytes, 0);
        assert_eq!(categories[1].file_count, 0);
    }

    #[test]
    fn programming_cache_processes_block_conservatively() {
        let processes = vec![RunningProcessInfo {
            name: "cargo.exe".into(),
            stem: "cargo".into(),
            command_paths: vec![normalize_path_key(r"C:\unrelated\workspace")],
            cwd: Some(normalize_path_key(r"C:\unrelated\workspace")),
        }];
        assert_eq!(
            running_process_blockers_for_path("rust-target", r"D:\project\target", &processes),
            vec!["cargo.exe"]
        );
    }

    #[test]
    fn programming_process_lists_cover_supported_toolchains() {
        let rust = watched_processes("rust-target").unwrap().0;
        assert!(rust.contains(&"cargo") && rust.contains(&"rustc"));

        let cpp = watched_processes("cpp-cache").unwrap().0;
        for process in [
            "cmake", "ninja", "msbuild", "cl", "clang", "gcc", "ccache", "sccache",
        ] {
            assert!(cpp.contains(&process), "{process}");
        }

        let python = watched_processes("python-cache").unwrap().0;
        for process in ["pip", "uv", "poetry", "pdm", "pytest", "mypy", "ruff"] {
            assert!(python.contains(&process), "{process}");
        }

        assert!(watched_processes("go-cache").unwrap().0.contains(&"go"));
        assert!(watched_processes("dotnet-cache")
            .unwrap()
            .0
            .contains(&"nuget"));
        assert!(watched_processes("java-cache")
            .unwrap()
            .0
            .contains(&"gradle"));
    }

    #[test]
    fn ordinary_access_warning_does_not_make_scan_incomplete() {
        let diagnostics = ScanDiagnostics::default();
        diagnostics.warn("C:\\protected: access denied");
        let result = finalize_scan_result(
            Vec::new(),
            &diagnostics,
            ColonyConfig {
                scout_workers: 1,
                engineer_workers: 2,
            },
            &std::collections::HashSet::new(),
        );
        assert!(result.complete);
        assert_eq!(result.skipped_paths, 1);
    }

    #[test]
    fn budget_truncation_makes_scan_incomplete() {
        let diagnostics = ScanDiagnostics::default();
        diagnostics.truncated.store(true, Ordering::Relaxed);
        let result = finalize_scan_result(
            Vec::new(),
            &diagnostics,
            ColonyConfig {
                scout_workers: 1,
                engineer_workers: 2,
            },
            &std::collections::HashSet::new(),
        );
        assert!(!result.complete);
    }

    #[test]
    fn application_processes_only_block_overlapping_cache_paths() {
        let processes = vec![RunningProcessInfo {
            name: "code.exe".into(),
            stem: "code".into(),
            command_paths: vec![normalize_path_key(r"C:\work\project")],
            cwd: Some(normalize_path_key(r"C:\work\project")),
        }];
        assert!(running_process_blockers_for_path(
            "app-cache",
            r"C:\Users\tester\AppData\Roaming\Code\Cache",
            &processes
        )
        .is_empty());

        let overlapping = vec![RunningProcessInfo {
            name: "code.exe".into(),
            stem: "code".into(),
            command_paths: Vec::new(),
            cwd: Some(normalize_path_key(
                r"C:\Users\tester\AppData\Roaming\Code\Cache",
            )),
        }];
        assert_eq!(
            running_process_blockers_for_path(
                "app-cache",
                r"C:\Users\tester\AppData\Roaming\Code\Cache",
                &overlapping
            ),
            vec!["code.exe"]
        );
    }

    #[test]
    fn process_argument_paths_extract_equals_values() {
        let keys = process_argument_path_keys(
            r#"--user-data-dir=\\?\C:\Users\tester\AppData\Local\Browser Data"#,
        );
        assert!(keys.contains(&normalize_path_key(
            r"C:\Users\tester\AppData\Local\Browser Data"
        )));
    }

    #[test]
    fn partial_path_progress_only_fills_unprocessed_items() {
        assert_eq!(remaining_path_items(10, 13, 8), 5);
        assert_eq!(remaining_path_items(10, 25, 8), 0);
        assert_eq!(remaining_path_items(10, 9, 8), 8);
    }

    #[test]
    fn cleanup_root_identity_rejects_replaced_directory() {
        let root = temp_root("identity");
        fs::create_dir_all(&root).unwrap();
        let detail = PathDetail {
            path: root.to_string_lossy().to_string(),
            size_bytes: 0,
            file_count: 0,
            ..path_detail_identity(&root)
        };
        assert!(revalidate_cleanup_root(&detail).is_some());

        let old = root.with_extension("old");
        fs::rename(&root, &old).unwrap();
        fs::create_dir_all(&root).unwrap();
        assert!(revalidate_cleanup_root(&detail).is_none());

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(old).unwrap();
    }

    #[test]
    fn cpp_build_directory_requires_markers_and_artifacts() {
        let root = temp_root("cpp");
        let build = root.join("build");
        fs::create_dir_all(build.join("CMakeFiles")).unwrap();
        fs::write(build.join("CMakeCache.txt"), b"cache").unwrap();
        assert!(is_verified_cpp_build_dir(&build, "build"));

        let fake = root.join("fake").join("build");
        fs::create_dir_all(&fake).unwrap();
        assert!(!is_verified_cpp_build_dir(&fake, "build"));

        let msbuild_root = root.join("msbuild");
        let obj = msbuild_root.join("obj");
        fs::create_dir_all(&obj).unwrap();
        fs::write(msbuild_root.join("sample.vcxproj"), b"project").unwrap();
        fs::write(obj.join("sample.pdb"), b"symbols").unwrap();
        assert!(is_verified_cpp_build_dir(&obj, "obj"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn python_project_cache_requires_project_marker() {
        let root = temp_root("python");
        let cache = root.join(".mypy_cache");
        fs::create_dir_all(&cache).unwrap();
        assert!(!is_verified_python_project_cache(&cache, ".mypy_cache"));
        fs::write(root.join("pyproject.toml"), b"[project]").unwrap();
        assert!(is_verified_python_project_cache(&cache, ".mypy_cache"));
        assert!(!is_verified_python_project_cache(
            &root.join(".venv"),
            ".venv"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_discovery_skips_dangerous_and_duplicate_roots() {
        let root = temp_root("project-skip");
        let appdata = root.join("AppData");
        let code = root.join("Code");
        fs::create_dir_all(&appdata).unwrap();
        fs::create_dir_all(&code).unwrap();
        assert!(should_skip_project_path(&root, &appdata, "appdata", 0));
        assert!(!should_skip_project_path(&root, &code, "code", 0));

        let mut paths = vec![
            code.clone(),
            PathBuf::from(code.to_string_lossy().to_uppercase()),
        ];
        deduplicate_paths(&mut paths);
        assert_eq!(paths.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_policy_rejects_files_modified_after_scan() {
        let root = temp_root("post-scan");
        fs::create_dir_all(&root).unwrap();
        let file_path = root.join("new.tmp");
        fs::write(&file_path, b"new").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let policy = CleanupFilePolicy {
            min_age_days: None,
            scanned_at: Some(SystemTime::UNIX_EPOCH),
        };
        assert_eq!(
            policy.file_decision(&metadata),
            CleanupPolicyDecision::ChangedAfterScan
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_policy_uses_scan_time_for_age_eligibility() {
        let root = temp_root("age-at-scan");
        fs::create_dir_all(&root).unwrap();
        let file_path = root.join("became-old.tmp");
        fs::write(&file_path, b"cache").unwrap();
        let scanned_at = SystemTime::now() + Duration::from_secs(1);
        let modified = scanned_at - Duration::from_secs(6 * 24 * 60 * 60);
        File::options()
            .write(true)
            .open(&file_path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        let policy = CleanupFilePolicy {
            min_age_days: Some(7),
            scanned_at: Some(scanned_at),
        };
        assert_eq!(
            policy.file_decision(&fs::metadata(&file_path).unwrap()),
            CleanupPolicyDecision::NotEligibleAtScan
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_policy_keeps_age_filter_at_delete_time() {
        let root = temp_root("delete-age");
        fs::create_dir_all(&root).unwrap();
        let file_path = root.join("recent.tmp");
        fs::write(&file_path, b"recent").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let policy = CleanupFilePolicy {
            min_age_days: Some(7),
            scanned_at: Some(SystemTime::now() + Duration::from_secs(1)),
        };
        assert_eq!(
            policy.file_decision(&metadata),
            CleanupPolicyDecision::NotEligibleAtScan
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_project_roots_are_canonicalized_and_deduplicated() {
        let root = temp_root("custom-root");
        fs::create_dir_all(&root).unwrap();
        let values = vec![
            root.to_string_lossy().to_string(),
            root.to_string_lossy().to_uppercase(),
            temp_root("missing").to_string_lossy().to_string(),
        ];
        let normalized = normalize_custom_project_roots(values);
        assert_eq!(normalized.len(), 1);
        assert!(normalized[0].is_dir());
        assert_eq!(
            directory_identity(&normalized[0]),
            directory_identity(&root)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_project_roots_are_scanned_before_broad_roots() {
        let root = temp_root("custom-priority");
        fs::create_dir_all(&root).unwrap();
        let roots = project_search_roots(std::slice::from_ref(&root));
        assert!(roots.first().is_some_and(|first| same_path(first, &root)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_fixed_volume_is_added_as_a_shallow_probe() {
        let fixed_a = temp_root("fixed-plan-a");
        let fixed_b = temp_root("fixed-plan-b");
        fs::create_dir_all(&fixed_a).unwrap();
        fs::create_dir_all(&fixed_b).unwrap();

        let plan = project_search_plan(&[], &[fixed_a.clone(), fixed_b.clone()]);
        for fixed in [&fixed_a, &fixed_b] {
            assert!(plan.iter().any(|root| {
                root.mode == ProjectScoutMode::FixedVolumeProbe && same_path(&root.path, fixed)
            }));
        }

        fs::remove_dir_all(fixed_a).unwrap();
        fs::remove_dir_all(fixed_b).unwrap();
    }

    #[test]
    fn deep_roots_take_precedence_over_duplicate_volume_probe_roots() {
        let root = temp_root("fixed-plan-priority");
        fs::create_dir_all(&root).unwrap();

        let plan = project_search_plan(std::slice::from_ref(&root), std::slice::from_ref(&root));
        let matches = plan
            .iter()
            .filter(|candidate| same_path(&candidate.path, &root))
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].mode, ProjectScoutMode::Deep);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_mount_points_are_deduplicated_by_directory_identity() {
        let root = temp_root("mount-dedup");
        fs::create_dir_all(&root).unwrap();
        let alias = PathBuf::from(root.to_string_lossy().to_uppercase());
        let mut roots = vec![root.clone(), alias];

        deduplicate_directory_roots(&mut roots);
        assert_eq!(roots.len(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixed_volume_probe_promotes_marked_project_to_deep_scan() {
        let volume = temp_root("fixed-probe");
        let project = volume.join("misc").join("sample");
        let target = project.join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            b"[package]\nname='sample'\nversion='0.1.0'\n",
        )
        .unwrap();

        let plan = vec![ProjectSearchRoot {
            path: volume.clone(),
            mode: ProjectScoutMode::FixedVolumeProbe,
        }];
        let roots = vec![volume.clone()];
        let queue = ProjectScoutQueue::from_plan(&plan);
        let diagnostics = Arc::new(ScanDiagnostics::default());
        let _guard = ActiveScanDiagnostics::install(diagnostics);
        let found = AtomicUsize::new(0);
        let volume_probe_tasks = AtomicUsize::new(1);
        let volume_probe_limit_reported = AtomicBool::new(false);
        let mut discovery = ProjectCacheDiscovery::default();

        while let Some(task) = queue.pop() {
            scan_project_cache_task(
                task,
                &roots,
                &queue,
                &found,
                &volume_probe_tasks,
                &volume_probe_limit_reported,
                &mut discovery,
            );
            queue.complete();
        }
        assert!(discovery.rust.iter().any(|path| same_path(path, &target)));

        fs::remove_dir_all(volume).unwrap();
    }

    #[test]
    fn nested_project_roots_are_assigned_once() {
        let root = temp_root("assigned-root");
        let child = root.join("projects");
        fs::create_dir_all(&child).unwrap();
        let roots = vec![root.clone(), child.clone()];

        assert!(is_separately_assigned_root(&root, &child, &roots));
        assert!(!is_separately_assigned_root(&child, &child, &roots));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scout_queue_does_not_finish_while_a_worker_can_enqueue_children() {
        let root = temp_root("scout-queue-race");
        let queue = Arc::new(ProjectScoutQueue::new(std::slice::from_ref(&root)));
        let task = queue.pop().unwrap();
        let waiting = queue.clone();
        let waiter = std::thread::spawn(move || waiting.pop());
        std::thread::sleep(Duration::from_millis(10));
        let child = ProjectScoutTask {
            root: root.clone(),
            dir: root.join("child"),
            depth: 1,
            mode: ProjectScoutMode::Deep,
        };
        queue.push(child.clone());
        queue.complete();
        let received = waiter.join().unwrap().unwrap();
        assert!(same_path(&received.dir, &child.dir));
        queue.complete();
        let _ = task;
    }

    #[test]
    fn directory_measure_cache_reuses_result_without_zeroing_later_categories() {
        let root = temp_root("measure-cache");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("cache.bin"), vec![7u8; 64]).unwrap();
        let cache = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let _cache_guard = ActiveScanDirectoryMeasures::install(cache.clone());

        let first = dir_size(&root);
        let second = dir_size(&root);

        assert_eq!(first, second);
        assert_eq!(first.1, 1);
        assert!(first.0 > 0);
        assert_eq!(cache.lock().unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scanner_handles_many_small_files_and_deep_directories() {
        let root = temp_root("performance");
        let mut leaf = root.clone();
        for depth in 0..20 {
            leaf = leaf.join(format!("level-{depth}"));
        }
        fs::create_dir_all(&leaf).unwrap();
        for index in 0..2_000 {
            fs::write(leaf.join(format!("file-{index}.tmp")), [index as u8]).unwrap();
        }

        let started = Instant::now();
        let (_size, count) = dir_size(&root);
        assert_eq!(count, 2_000);
        assert!(started.elapsed() < Duration::from_secs(10));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sandbox_cleanup_deletes_only_scanned_files() {
        let root = temp_root("sandbox-clean");
        fs::create_dir_all(&root).unwrap();
        let scanned = root.join("scanned.tmp");
        let newer = root.join("newer.tmp");
        fs::write(&scanned, b"scanned").unwrap();
        fs::write(&newer, b"newer").unwrap();

        let scanned_time = SystemTime::now() - Duration::from_secs(30);
        let future_time = SystemTime::now() + Duration::from_secs(30);
        File::options()
            .write(true)
            .open(&scanned)
            .unwrap()
            .set_times(FileTimes::new().set_modified(scanned_time))
            .unwrap();
        File::options()
            .write(true)
            .open(&newer)
            .unwrap()
            .set_times(FileTimes::new().set_modified(future_time))
            .unwrap();

        let policy = CleanupFilePolicy {
            min_age_days: None,
            scanned_at: Some(SystemTime::now()),
        };
        let mut deleted = 0;
        for entry in fs::read_dir(&root).unwrap().flatten() {
            let metadata = entry.metadata().unwrap();
            if policy.file_decision(&metadata) == CleanupPolicyDecision::Allow {
                fs::remove_file(entry.path()).unwrap();
                deleted += 1;
            }
        }
        assert_eq!(deleted, 1);
        assert!(!scanned.exists());
        assert!(newer.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_policy_rejects_directories_created_after_scan() {
        let root = temp_root("new-directory");
        fs::create_dir_all(&root).unwrap();
        let policy = CleanupFilePolicy {
            min_age_days: None,
            scanned_at: Some(SystemTime::UNIX_EPOCH),
        };
        assert_eq!(
            policy.directory_decision(&fs::metadata(&root).unwrap()),
            CleanupPolicyDecision::ChangedAfterScan
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_policy_rejects_new_file_with_backdated_modified_time() {
        let root = temp_root("backdated-new-file");
        fs::create_dir_all(&root).unwrap();
        let scanned_at = SystemTime::now() - Duration::from_secs(10);
        let file_path = root.join("new.tmp");
        fs::write(&file_path, b"new").unwrap();
        File::options()
            .write(true)
            .open(&file_path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(scanned_at - Duration::from_secs(10)))
            .unwrap();
        let policy = CleanupFilePolicy {
            min_age_days: None,
            scanned_at: Some(scanned_at),
        };
        assert_eq!(
            policy.file_decision(&fs::metadata(&file_path).unwrap()),
            CleanupPolicyDecision::ChangedAfterScan
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cargo_config_target_dir_is_discovered() {
        let root = temp_root("cargo-config");
        let cargo_dir = root.join(".cargo");
        let target = root.join("shared-target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::create_dir_all(&cargo_dir).unwrap();
        fs::write(
            cargo_dir.join("config.toml"),
            b"[build]\ntarget-dir = \"shared-target\"\n",
        )
        .unwrap();
        let discovered = cargo_target_dir_from_config(&cargo_dir.join("config.toml")).unwrap();
        assert!(same_path(&discovered, &target));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cargo_config_absolute_target_dir_is_preserved() {
        let root = temp_root("cargo-config-absolute");
        let cargo_dir = root.join(".cargo");
        let target = temp_root("cargo-config-external-target");
        fs::create_dir_all(target.join("release")).unwrap();
        fs::create_dir_all(&cargo_dir).unwrap();
        fs::write(
            cargo_dir.join("config.toml"),
            format!(
                "[build]\ntarget-dir = {:?}\n",
                target.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let discovered = cargo_target_dir_from_config(&cargo_dir.join("config.toml")).unwrap();
        assert!(same_path(&discovered, &target));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[test]
    fn project_level_cargo_target_dir_is_discovered_without_parent_manifest() {
        let root = temp_root("cargo-project-target");
        let cargo_dir = root.join(".cargo");
        let crate_dir = root.join("src-tauri");
        let target = root.join("target");
        fs::create_dir_all(target.join("debug")).unwrap();
        fs::create_dir_all(&crate_dir).unwrap();
        fs::create_dir_all(&cargo_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            b"[package]\nname='sample'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            cargo_dir.join("config.toml"),
            b"[build]\ntarget-dir = \"target\"\n",
        )
        .unwrap();

        assert!(is_verified_rust_target(&target, "target"));
        let roots = vec![root.clone()];
        let queue = ProjectScoutQueue::new(&roots);
        let diagnostics = Arc::new(ScanDiagnostics::default());
        let _guard = ActiveScanDiagnostics::install(diagnostics);
        let found = AtomicUsize::new(0);
        let volume_probe_tasks = AtomicUsize::new(0);
        let volume_probe_limit_reported = AtomicBool::new(false);
        let mut discovery = ProjectCacheDiscovery::default();
        let task = queue.pop().unwrap();
        scan_project_cache_task(
            task,
            &roots,
            &queue,
            &found,
            &volume_probe_tasks,
            &volume_probe_limit_reported,
            &mut discovery,
        );
        queue.complete();
        assert!(discovery.rust.iter().any(|path| same_path(path, &target)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_depth_limit_is_reported() {
        let root = temp_root("depth-warning");
        let child = root.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("file.tmp"), b"data").unwrap();
        let mut result = DirectoryMeasure::default();
        let mut visited = std::collections::HashSet::new();
        dir_size_recursive(&root, &mut result, &mut visited, 0, 0, None);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("最大扫描深度")));
        fs::remove_dir_all(root).unwrap();
    }
}

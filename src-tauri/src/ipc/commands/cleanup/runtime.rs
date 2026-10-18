use super::*;

pub(super) type FileIdentity = (u64, u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct DirectoryMeasureKey {
    pub(super) path: String,
    pub(super) min_age_days: Option<u32>,
}

#[derive(Default)]
pub(super) struct DirectoryMeasureCellState {
    pub(super) result: Mutex<Option<(u64, u64)>>,
    pub(super) ready: Condvar,
}

impl DirectoryMeasureCellState {
    pub(super) fn complete(&self, result: (u64, u64)) {
        let mut value = self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if value.is_none() {
            *value = Some(result);
            self.ready.notify_all();
        }
    }

    pub(super) fn wait(&self) -> (u64, u64) {
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

pub(super) type DirectoryMeasureCell = Arc<DirectoryMeasureCellState>;
pub(super) type SharedDirectoryMeasures =
    Arc<Mutex<std::collections::HashMap<DirectoryMeasureKey, DirectoryMeasureCell>>>;

pub(super) static CLEANUP_STATE: AtomicU8 = AtomicU8::new(CleanupTaskKind::Idle as u8);
pub(super) static LAST_SCAN: OnceLock<Mutex<Option<CachedScan>>> = OnceLock::new();
pub(super) static NEXT_SCAN_ID: AtomicU64 = AtomicU64::new(1);
pub(super) static SCAN_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
pub(super) static ACTIVE_SCAN_DIAGNOSTICS: OnceLock<Mutex<Option<Arc<ScanDiagnostics>>>> =
    OnceLock::new();
pub(super) static ACTIVE_SCAN_APP: OnceLock<Mutex<Option<AppHandle>>> = OnceLock::new();
pub(super) static ACTIVE_SCAN_DIRECTORY_MEASURES: OnceLock<Mutex<Option<SharedDirectoryMeasures>>> =
    OnceLock::new();
pub(super) static ACTIVE_SCAN_FILE_QUEUE: OnceLock<Mutex<Option<Arc<FileScanQueue>>>> =
    OnceLock::new();
pub(super) const CLEANUP_PROGRESS_EVENT: &str = "cleanup:progress";
pub(super) const CLEANUP_SCAN_PROGRESS_EVENT: &str = "cleanup:scan-progress";
pub(super) const SCAN_TTL: Duration = Duration::from_secs(10 * 60);
pub(super) const DIRECTORY_SCAN_MAX_DEPTH: u32 = 64;
pub(super) const MAX_SCAN_ENTRIES: u64 = 4_000_000;
pub(super) const MAX_SCAN_DURATION: Duration = Duration::from_secs(10 * 60);
pub(super) const MAX_SCAN_WARNINGS: usize = 80;
pub(super) const MAX_PROJECT_CACHE_PATHS: usize = 20_000;
pub(super) const FIXED_VOLUME_PROBE_DEPTH: u32 = 6;
pub(super) const MAX_FIXED_VOLUME_PROBE_TASKS: usize = 50_000;
pub(super) const COMMAND_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const HOTSPOT_SCHEMA_VERSION: u32 = 1;
pub(super) const HOTSPOT_MAX_ENTRIES: usize = 2_048;
pub(super) const HOTSPOT_MAX_MISSES: u32 = 3;

#[derive(Clone, Copy)]
pub(super) struct ColonyConfig {
    pub(super) scout_workers: usize,
    pub(super) engineer_workers: usize,
}

impl ColonyConfig {
    pub(super) fn detect() -> Self {
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
pub(super) struct HotspotEntry {
    pub(super) path: String,
    pub(super) category_id: String,
    pub(super) matched_rule: String,
    pub(super) last_seen_ms: i64,
    pub(super) last_scanned_ms: i64,
    pub(super) size_bytes: u64,
    pub(super) file_count: u64,
    pub(super) miss_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HotspotIndex {
    pub(super) schema_version: u32,
    pub(super) entries: Vec<HotspotEntry>,
}

impl Default for HotspotIndex {
    fn default() -> Self {
        Self {
            schema_version: HOTSPOT_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

pub(super) struct FileScanJob {
    pub(super) path: PathBuf,
    pub(super) min_age_days: Option<u32>,
    pub(super) measurement: DirectoryMeasureCell,
}

pub(super) struct FileScanQueue {
    pub(super) sender: std::sync::mpsc::Sender<FileScanJob>,
    pub(super) workers: Vec<std::thread::JoinHandle<()>>,
}

impl FileScanQueue {
    pub(super) fn start(worker_count: usize) -> Self {
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

pub(super) struct ActiveScanDiagnostics;

impl ActiveScanDiagnostics {
    pub(super) fn install(diagnostics: Arc<ScanDiagnostics>) -> Self {
        if let Ok(mut active) = active_scan_diagnostics().lock() {
            *active = Some(diagnostics);
        }
        Self
    }
}

pub(super) struct ActiveScanApp;

impl ActiveScanApp {
    pub(super) fn install(app: AppHandle) -> Self {
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

pub(super) struct ActiveScanDirectoryMeasures;

impl ActiveScanDirectoryMeasures {
    pub(super) fn install(measures: SharedDirectoryMeasures) -> Self {
        if let Ok(mut active) = active_scan_directory_measures().lock() {
            *active = Some(measures);
        }
        Self
    }
}

pub(super) struct ActiveScanFileQueue;

impl ActiveScanFileQueue {
    pub(super) fn install(queue: Arc<FileScanQueue>) -> Self {
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

pub(super) fn active_scan_diagnostics() -> &'static Mutex<Option<Arc<ScanDiagnostics>>> {
    ACTIVE_SCAN_DIAGNOSTICS.get_or_init(|| Mutex::new(None))
}

pub(super) fn active_scan_app() -> &'static Mutex<Option<AppHandle>> {
    ACTIVE_SCAN_APP.get_or_init(|| Mutex::new(None))
}

pub(super) fn active_scan_directory_measures() -> &'static Mutex<Option<SharedDirectoryMeasures>> {
    ACTIVE_SCAN_DIRECTORY_MEASURES.get_or_init(|| Mutex::new(None))
}

pub(super) fn active_scan_file_queue() -> &'static Mutex<Option<Arc<FileScanQueue>>> {
    ACTIVE_SCAN_FILE_QUEUE.get_or_init(|| Mutex::new(None))
}

pub(super) fn current_scan_file_queue() -> Option<Arc<FileScanQueue>> {
    active_scan_file_queue()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

pub(super) fn current_scan_directory_measures() -> Option<SharedDirectoryMeasures> {
    active_scan_directory_measures()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

pub(super) fn with_scan_diagnostics(callback: impl FnOnce(&ScanDiagnostics)) {
    if let Ok(active) = active_scan_diagnostics().lock() {
        if let Some(diagnostics) = active.as_ref() {
            callback(diagnostics);
        }
    }
}

pub(super) fn current_scan_diagnostics() -> Option<Arc<ScanDiagnostics>> {
    active_scan_diagnostics()
        .lock()
        .ok()
        .and_then(|active| active.as_ref().cloned())
}

#[derive(Clone)]
pub(super) struct CachedScan {
    pub(super) result: ScanResult,
    pub(super) created_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CleanupTaskKind {
    Idle = 0,
    Scanning = 1,
    Cleaning = 2,
    LargeScanning = 3,
}

pub(super) struct CleanupTaskGuard {
    pub(super) kind: CleanupTaskKind,
}

impl CleanupTaskGuard {
    pub(super) fn acquire(kind: CleanupTaskKind) -> Result<Self, IpcError> {
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

pub(super) fn cleanup_state_name(value: u8) -> &'static str {
    match value {
        value if value == CleanupTaskKind::Scanning as u8 => "scanning",
        value if value == CleanupTaskKind::Cleaning as u8 => "cleaning",
        value if value == CleanupTaskKind::LargeScanning as u8 => "large-scanning",
        _ => "idle",
    }
}

pub(super) fn scan_cache() -> &'static Mutex<Option<CachedScan>> {
    LAST_SCAN.get_or_init(|| Mutex::new(None))
}

pub(super) fn cached_scan(scan_id: &str) -> Option<ScanResult> {
    scan_cache().lock().ok().and_then(|guard| {
        guard.as_ref().and_then(|cached| {
            (cached.result.scan_id == scan_id && cached.created_at.elapsed() <= SCAN_TTL)
                .then(|| cached.result.clone())
        })
    })
}

pub(super) fn take_cached_scan(scan_id: &str) -> Option<ScanResult> {
    let mut guard = scan_cache().lock().ok()?;
    let matches = guard.as_ref().is_some_and(|cached| {
        cached.result.scan_id == scan_id && cached.created_at.elapsed() <= SCAN_TTL
    });
    matches.then(|| guard.take().expect("validated cached scan").result)
}

pub(super) fn clear_scan_cache() {
    if let Ok(mut guard) = scan_cache().lock() {
        *guard = None;
    }
}

pub(super) fn store_scan(scan: &ScanResult) {
    if let Ok(mut guard) = scan_cache().lock() {
        *guard = Some(CachedScan {
            result: scan.clone(),
            created_at: Instant::now(),
        });
    }
}

pub(super) fn scan_is_cleanable(scan: &ScanResult) -> bool {
    scan.complete && !scan.cancelled && !scan.scan_id.is_empty()
}

pub(super) fn load_hotspot_index() -> HotspotIndex {
    let path = crate::paths::cleanup_hotspots_file();
    let Ok(bytes) = std::fs::read(path) else {
        return HotspotIndex::default();
    };
    serde_json::from_slice::<HotspotIndex>(&bytes)
        .ok()
        .filter(|index| index.schema_version == HOTSPOT_SCHEMA_VERSION)
        .unwrap_or_default()
}

pub(super) fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<String, IpcError> {
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
pub(super) fn replace_file(source: &Path, target: &Path) -> Result<(), IpcError> {
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
pub(super) fn replace_file(source: &Path, target: &Path) -> Result<(), IpcError> {
    std::fs::rename(source, target).map_err(crate::error::AppError::Io)?;
    Ok(())
}

pub(super) fn save_hotspot_index(index: &HotspotIndex) {
    let _ = write_json_atomic(&crate::paths::cleanup_hotspots_file(), index);
}

pub(super) fn hotspot_paths(index: &HotspotIndex) -> Vec<PathBuf> {
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

pub(super) fn update_hotspot_index(
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

use super::*;
use std::collections::BTreeMap;

// Server-owned, bounded inventory. Never accepted from the webview. Keeping the
// inventory with the scan also prevents a later scan from changing a clean job.
const MAX_PLAN_BYTES: usize = 192 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct PlannedFile {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub modified: SystemTime,
    pub created: SystemTime,
    pub length: u64,
    pub allocated: u64,
}

#[derive(Debug, Default)]
pub(super) struct FilePlan {
    pub files: BTreeMap<String, PlannedFile>,
    bytes: usize,
}

impl FilePlan {
    pub fn record(&mut self, file: PlannedFile) -> bool {
        let key = normalize_path_key(&file.path.to_string_lossy());
        if self.files.contains_key(&key) {
            return true;
        }
        let cost = key.len() + file.path.as_os_str().len() * 2 + 192;
        if self.bytes.saturating_add(cost) > MAX_PLAN_BYTES {
            return false;
        }
        self.bytes += cost;
        self.files.insert(key, file);
        true
    }

    pub fn under<'a>(&'a self, root: &Path) -> impl Iterator<Item = &'a PlannedFile> + 'a {
        let prefix = format!("{}\\", normalize_path_key(&root.to_string_lossy()));
        self.files
            .range(prefix.clone()..)
            .take_while(move |(key, _)| key.starts_with(&prefix))
            .map(|(_, file)| file)
    }
}

pub(super) fn record_scan_file(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Option<(u64, u64, u64)> {
    let details = if metadata.is_file() && !metadata_is_reparse_point(metadata) {
        file_details(path, metadata)
    } else {
        None
    };
    let Some(diagnostics) = current_scan_diagnostics() else {
        return details;
    };
    let file = (|| {
        if !metadata.is_file() || metadata_is_reparse_point(metadata) {
            return None;
        }
        let (volume, id, allocated) = details?;
        Some(PlannedFile {
            path: path.to_path_buf(),
            identity: (volume, id),
            modified: metadata.modified().ok()?,
            created: metadata.created().ok()?,
            length: metadata.len(),
            allocated,
        })
    })();
    match file {
        Some(file) => {
            if !diagnostics
                .plan
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record(file)
            {
                if !diagnostics.truncated.swap(true, Ordering::Relaxed) {
                    diagnostics.warn("文件清单达到内存上限，请缩小项目目录范围后重新扫描");
                }
            }
        }
        None => diagnostics.warn(format!(
            "{}: 无法记录文件身份，清理时将保留",
            path.display()
        )),
    }
    details
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeleteOutcome {
    Deleted(u64),
    Changed,
    Missing,
}

/// Lock every ancestor against replacement, then validate and delete using one
/// file handle. A successful check of a path followed by remove_file(path) is
/// insufficient: another process can rename an ancestor between those steps.
#[cfg(windows)]
pub(super) fn delete_planned_file(
    root: &PathDetail,
    expected: &PlannedFile,
) -> std::io::Result<DeleteOutcome> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileDispositionInfo, GetFileInformationByHandle, SetFileInformationByHandle,
        BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let root_path = Path::new(&root.path);
    if !path_is_within_key(
        &normalize_path_key(&expected.path.to_string_lossy()),
        &normalize_path_key(&root.path),
    ) {
        return Ok(DeleteOutcome::Changed);
    }
    let identity = |file: &std::fs::File| -> std::io::Result<FileIdentity> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| std::io::Error::last_os_error())?;
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    };
    let mut locks = Vec::new();
    let ancestors: Vec<_> = expected
        .path
        .parent()
        .into_iter()
        .flat_map(Path::ancestors)
        .collect();
    let mut root_verified = false;
    for directory in ancestors.into_iter().rev() {
        if directory.as_os_str().is_empty() {
            continue;
        }
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(directory);
        let handle = match handle {
            Ok(handle) => handle,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DeleteOutcome::Missing)
            }
            Err(error) => return Err(error),
        };
        let meta = handle.metadata()?;
        if !meta.is_dir() || metadata_is_reparse_point(&meta) {
            return Ok(DeleteOutcome::Changed);
        }
        if same_path(directory, root_path) {
            if Some(identity(&handle)?) != root.volume_serial.zip(root.file_id) {
                return Ok(DeleteOutcome::Changed);
            }
            root_verified = true;
        }
        locks.push(handle);
    }
    if !root_verified {
        return Ok(DeleteOutcome::Changed);
    }
    // DELETE | FILE_READ_ATTRIBUTES; no write/delete sharing, so writers and
    // replacement cannot race the validation. Busy files remain untouched.
    let handle = match std::fs::OpenOptions::new()
        .access_mode(0x0001_0000 | FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&expected.path)
    {
        Ok(handle) => handle,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DeleteOutcome::Missing)
        }
        Err(error) => return Err(error),
    };
    let meta = handle.metadata()?;
    if !meta.is_file()
        || metadata_is_reparse_point(&meta)
        || identity(&handle)? != expected.identity
        || meta.len() != expected.length
        || meta.modified()? != expected.modified
        || meta.created()? != expected.created
    {
        return Ok(DeleteOutcome::Changed);
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(handle.as_raw_handle()), &mut info) }
        .map_err(|_| std::io::Error::last_os_error())?;
    let reclaimed = if info.nNumberOfLinks == 1 {
        expected.allocated
    } else {
        0
    };
    let disposition = FILE_DISPOSITION_INFO {
        DeleteFile: true.into(),
    };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(handle.as_raw_handle()),
            FileDispositionInfo,
            &disposition as *const _ as _,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())?;
    drop(handle);
    drop(locks);
    Ok(DeleteOutcome::Deleted(reclaimed))
}

#[cfg(not(windows))]
pub(super) fn delete_planned_file(
    _root: &PathDetail,
    _expected: &PlannedFile,
) -> std::io::Result<DeleteOutcome> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "清理执行器仅支持 Windows",
    ))
}

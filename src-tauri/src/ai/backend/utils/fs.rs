use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn atomic_write(path: &Path, content: &str, label: &str) -> Result<(), String> {
    atomic_write_bytes(path, content.as_bytes(), label)
}

pub fn atomic_write_bytes(path: &Path, content: &[u8], label: &str) -> Result<(), String> {
    atomic_write_with(path, label, |file| {
        file.write_all(content)
            .map_err(|e| format!("写入 {label} 临时文件失败: {e}"))
    })
}

pub fn atomic_write_with(
    path: &Path,
    label: &str,
    write: impl FnOnce(&mut fs::File) -> Result<(), String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {label} 目录失败: {e}"))?;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let tmp_path = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("file"),
        timestamp
    ));

    let mut temporary = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .map_err(|e| format!("创建 {label} 临时文件失败: {e}"))?;
    if let Err(error) = write(&mut temporary) {
        drop(temporary);
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    if let Err(error) = temporary.sync_all() {
        drop(temporary);
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("同步 {label} 临时文件失败: {error}"));
    }
    drop(temporary);

    replace_file(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("替换 {label} 失败: {e}")
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(std::io::Error::other)
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_atomic_write_keeps_target_and_removes_temporary_file() {
        let dir =
            std::env::temp_dir().join(format!("syspulse-atomic-write-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("export.md");
        fs::write(&target, "stable").unwrap();

        let result = atomic_write_with(&target, "测试导出", |file| {
            file.write_all(b"partial").unwrap();
            Err("模拟写入失败".to_string())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "stable");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }
}

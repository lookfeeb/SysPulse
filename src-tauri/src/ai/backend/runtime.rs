use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 在任何数据库或 OAuth 操作前设置宿主应用专用数据目录。
pub fn set_data_dir(path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref().to_path_buf();
    if let Some(current) = DATA_DIR.get() {
        return if current == &path {
            Ok(())
        } else {
            Err(format!(
                "AI 工具数据目录已设置为 {}，不能改为 {}",
                current.display(),
                path.display()
            ))
        };
    }
    DATA_DIR
        .set(path)
        .map_err(|_| "AI 工具数据目录初始化失败".to_string())
}

pub fn data_dir() -> PathBuf {
    DATA_DIR.get().cloned().unwrap_or_else(|| {
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir)
            .join("SysPulse")
            .join("ai-tools")
    })
}

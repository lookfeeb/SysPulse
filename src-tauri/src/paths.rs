use std::path::PathBuf;

const APP_DIR: &str = "SysPulse";

/// %APPDATA%\SysPulse\
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

/// %LOCALAPPDATA%\SysPulse\
pub fn data_local_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn db_file() -> PathBuf {
    config_dir().join("traffic.db")
}

pub fn logs_dir() -> PathBuf {
    data_local_dir().join("logs")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(logs_dir())?;
    Ok(())
}

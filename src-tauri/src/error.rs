use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("config: {0}")]
    Config(String),

    #[error("toml parse: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("db pool: {0}")]
    DbPool(#[from] r2d2::Error),

    #[error("collector {what}: {msg}")]
    Collect { what: &'static str, msg: String },

    #[error("invalid argument: {0}")]
    Invalid(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("tauri: {0}")]
    Tauri(#[from] tauri::Error),

    #[cfg(windows)]
    #[error("windows: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl From<AppError> for IpcError {
    fn from(e: AppError) -> Self {
        let code = match &e {
            AppError::Config(_) | AppError::TomlDe(_) | AppError::TomlSer(_) => "CONFIG",
            AppError::Db(_) | AppError::DbPool(_) => "DB",
            #[cfg(windows)]
            AppError::Windows(_) => "WINDOWS",
            AppError::Collect { .. } => "COLLECT",
            AppError::Invalid(_) => "INVALID",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Io(_) => "IO",
            AppError::Tauri(_) => "TAURI",
            AppError::Other(_) => "OTHER",
        };
        IpcError {
            code: code.to_string(),
            message: e.to_string(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        IpcError::from(AppError::Other(self.to_string())).serialize(ser)
    }
}

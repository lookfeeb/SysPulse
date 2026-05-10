use crate::error::{AppError, IpcError};
use crate::paths;
use serde::Serialize;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub config_dir: String,
    pub logs_dir: String,
    pub db_file: String,
}

#[tauri::command]
#[specta::specta]
pub fn get_app_info(app: AppHandle) -> Result<AppInfo, IpcError> {
    let pkg = app.package_info();
    Ok(AppInfo {
        name: pkg.name.clone(),
        version: pkg.version.to_string(),
        os: os_name(),
        arch: std::env::consts::ARCH.into(),
        config_dir: paths::config_dir().to_string_lossy().to_string(),
        logs_dir: paths::logs_dir().to_string_lossy().to_string(),
        db_file: paths::db_file().to_string_lossy().to_string(),
    })
}

#[derive(serde::Deserialize, specta::Type)]
pub struct OpenPathArgs {
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub fn open_path(app: AppHandle, args: OpenPathArgs) -> Result<(), IpcError> {
    use tauri_plugin_opener::OpenerExt;

    // Restrict to known directories under our app data tree.
    let p = std::path::PathBuf::from(&args.path);
    let cfg_dir = paths::config_dir();
    let log_dir = paths::logs_dir();
    if !p.starts_with(&cfg_dir) && !p.starts_with(&log_dir) {
        return Err(AppError::Invalid(format!(
            "open_path refused: {} is outside app data",
            args.path
        ))
        .into());
    }
    app.opener()
        .reveal_item_in_dir(p)
        .map_err(|e| AppError::Config(format!("reveal failed: {e}")))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn quit_app(app: AppHandle) -> Result<(), IpcError> {
    crate::app::quit_gracefully(app).await;
    Ok(())
}

fn os_name() -> String {
    #[cfg(windows)]
    {
        windows_version_name()
    }
    #[cfg(not(windows))]
    {
        std::env::consts::OS.into()
    }
}

#[cfg(windows)]
fn windows_version_name() -> String {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct RtlOsVersionInfo {
        dwOSVersionInfoSize: u32,
        dwMajorVersion: u32,
        dwMinorVersion: u32,
        dwBuildNumber: u32,
        dwPlatformId: u32,
        szCSDVersion: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut RtlOsVersionInfo) -> i32;
    }

    let mut info = RtlOsVersionInfo {
        dwOSVersionInfoSize: std::mem::size_of::<RtlOsVersionInfo>() as u32,
        dwMajorVersion: 0,
        dwMinorVersion: 0,
        dwBuildNumber: 0,
        dwPlatformId: 0,
        szCSDVersion: [0; 128],
    };

    let ok = unsafe { RtlGetVersion(&mut info) } >= 0;
    if !ok {
        return "Windows".into();
    }

    match (info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber) {
        (10, 0, build) if build >= 22_000 => "Windows 11".into(),
        (10, 0, _) => "Windows 10".into(),
        (6, 3, _) => "Windows 8.1".into(),
        (6, 2, _) => "Windows 8".into(),
        (6, 1, _) => "Windows 7".into(),
        (major, minor, build) => format!("Windows {major}.{minor}.{build}"),
    }
}

use crate::error::{AppError, IpcError};
use winreg::enums::*;
use winreg::RegKey;

/// Registry key path for current-user auto-start programs.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Value name in the registry.
const VALUE_NAME: &str = "SysPulse";

// ============================================================================
// IPC commands — auto-start via registry (HKCU\...\Run).
//
// This is fast (no subprocess spawn), works without admin for HKCU,
// and the program's manifest `requireAdministrator` ensures Windows
// will elevate it at logon (showing UAC if needed, or auto-elevating
// for admin accounts with UAC disabled).
// ============================================================================

#[tauri::command]
#[specta::specta]
pub fn autostart_is_enabled() -> Result<bool, IpcError> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey(RUN_KEY)
        .map_err(|e| AppError::Config(format!("failed to open Run key: {e}")))?;
    Ok(run_key.get_value::<String, _>(VALUE_NAME).is_ok())
}

#[tauri::command]
#[specta::specta]
pub fn autostart_enable() -> Result<(), IpcError> {
    let exe_path = std::env::current_exe()
        .map_err(|e| AppError::Config(format!("failed to get exe path: {e}")))?;
    let exe_str = exe_path.to_string_lossy().to_string();

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu
        .create_subkey(RUN_KEY)
        .map_err(|e| AppError::Config(format!("failed to open Run key: {e}")))?;
    run_key
        .set_value(VALUE_NAME, &exe_str)
        .map_err(|e| AppError::Config(format!("failed to set autostart value: {e}")))?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn autostart_disable() -> Result<(), IpcError> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_WRITE)
        .map_err(|e| AppError::Config(format!("failed to open Run key: {e}")))?;
    // Ignore error if value doesn't exist (idempotent).
    let _ = run_key.delete_value(VALUE_NAME);
    Ok(())
}

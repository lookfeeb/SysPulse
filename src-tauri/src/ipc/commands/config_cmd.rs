use crate::app::AppState;
use crate::config::{AppConfig, OverlayConfig};
use crate::error::IpcError;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn get_config(state: State<'_, AppState>) -> Result<AppConfig, IpcError> {
    Ok(state.config.snapshot())
}

#[derive(serde::Deserialize, specta::Type)]
pub struct SetConfigArgs {
    pub patch: serde_json::Value,
}

#[tauri::command]
#[specta::specta]
pub fn set_config(
    state: State<'_, AppState>,
    args: SetConfigArgs,
) -> Result<AppConfig, IpcError> {
    let new_cfg = state.config.apply_patch(args.patch)?;
    Ok(new_cfg)
}

#[tauri::command]
#[specta::specta]
pub fn reset_config(state: State<'_, AppState>) -> Result<AppConfig, IpcError> {
    let new_cfg = state.config.reset()?;
    Ok(new_cfg)
}

#[tauri::command]
#[specta::specta]
pub fn get_overlay_config(state: State<'_, AppState>) -> Result<OverlayConfig, IpcError> {
    Ok(state.config.snapshot().overlay)
}

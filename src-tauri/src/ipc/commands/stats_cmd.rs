use crate::app::AppState;
use crate::error::IpcError;
use crate::monitor::Snapshot;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn get_realtime_stats(state: State<'_, AppState>) -> Result<Option<Snapshot>, IpcError> {
    Ok(state.last_snapshot.read().clone())
}

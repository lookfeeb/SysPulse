use crate::app::AppState;
use crate::error::IpcError;
use crate::storage::queries::{query_history, DailyTraffic, HistoryQuery};
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
#[specta::specta]
pub fn query_traffic_history(
    state: State<'_, AppState>,
    query: HistoryQuery,
) -> Result<Vec<DailyTraffic>, IpcError> {
    Ok(query_history(state.store.pool(), &query)?)
}

#[derive(serde::Deserialize, specta::Type)]
pub struct ExportArgs {
    pub query: HistoryQuery,
    pub path: String,
}

#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub saved_to: String,
    pub rows: usize,
}

#[tauri::command]
#[specta::specta]
pub fn export_traffic_csv(
    state: State<'_, AppState>,
    args: ExportArgs,
) -> Result<ExportResult, IpcError> {
    let rows = query_history(state.store.pool(), &args.query)?;
    let path = PathBuf::from(&args.path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(crate::error::AppError::Io)?;
    }
    let mut out = String::new();
    // UTF-8 BOM so Excel reads it correctly.
    out.push('\u{FEFF}');
    out.push_str("date,iface_luid,bytes_recv,bytes_sent\n");
    for r in &rows {
        let iface = r.iface.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{},{},{},{}\n",
            r.date, iface, r.bytes_recv, r.bytes_sent
        ));
    }
    std::fs::write(&path, out).map_err(crate::error::AppError::Io)?;
    Ok(ExportResult {
        saved_to: path.to_string_lossy().to_string(),
        rows: rows.len(),
    })
}

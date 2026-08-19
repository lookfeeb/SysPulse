pub mod backend;
pub mod models;

use backend::services::session_storage::SessionStorage;
use std::sync::Arc;
use tauri::{App, Manager};

pub struct AiState {
    pub sessions: Arc<SessionStorage>,
}

pub fn setup(app: &mut App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data_dir = crate::paths::data_local_dir().join("ai-tools");
    std::fs::create_dir_all(&data_dir)?;
    backend::runtime::set_data_dir(&data_dir).map_err(std::io::Error::other)?;
    backend::db::init_global().map_err(std::io::Error::other)?;

    let sessions = Arc::new(SessionStorage::new()?);
    app.manage(AiState { sessions });

    tauri::async_runtime::spawn(async {
        match backend::mcp_proxy::start_proxy().await {
            Ok(_) => {
                match backend::commands::mcp_oauth_cmd::reconcile_mcp_oauth_proxy_configs().await {
                    Ok(repaired) if repaired > 0 => {
                        tracing::info!(repaired, "AI MCP proxy configurations repaired");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%error, "AI MCP proxy configuration repair incomplete");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "AI MCP proxy failed to start");
            }
        }
    });
    backend::tasks::mcp_token_refresh::start_mcp_token_refresh_loop(app.handle().clone());

    Ok(())
}

use crate::ai::models::{
    AiMcpOAuthStatus, AiMcpOverview, AiMcpServerItem, AiSessionPage, AiSessionTree,
};
use crate::ai::AiState;
use crate::error::IpcError;
use serde::Deserialize;
use specta::Type;
use std::path::PathBuf;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

fn ai_error(context: &str, error: impl std::fmt::Display) -> IpcError {
    IpcError {
        code: "AI_TOOLS".to_string(),
        message: format!("{context}: {error}"),
    }
}

async fn run_session_task<T, F>(context: &'static str, task: F) -> Result<T, IpcError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| ai_error(context, error))?
        .map_err(|error| ai_error(context, error))
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpClientArgs {
    pub client: String,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpServerArgs {
    pub client: String,
    pub name: String,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiToggleMcpServerArgs {
    pub client: String,
    pub name: String,
    pub disabled: bool,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiCopyMcpServerArgs {
    pub from_client: String,
    pub to_client: String,
    pub name: String,
    pub overwrite: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn ai_get_mcp_overview() -> Result<AiMcpOverview, IpcError> {
    crate::ai::backend::commands::mcp_cmd::get_mcp_clients_overview()
        .await
        .map(AiMcpOverview::from)
        .map_err(|error| ai_error("读取 MCP 概览失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_get_mcp_servers(args: AiMcpClientArgs) -> Result<Vec<AiMcpServerItem>, IpcError> {
    crate::ai::backend::commands::mcp_cmd::get_mcp_config_by_client(args.client)
        .await
        .map(|items| items.into_iter().map(AiMcpServerItem::from).collect())
        .map_err(|error| ai_error("读取 MCP 配置失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_discover_mcp_servers(args: AiMcpClientArgs) -> Result<Vec<String>, IpcError> {
    crate::ai::backend::commands::mcp_cmd::discover_and_import_mcp_servers(args.client)
        .await
        .map_err(|error| ai_error("扫描 MCP 配置失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_toggle_mcp_server(args: AiToggleMcpServerArgs) -> Result<(), IpcError> {
    crate::ai::backend::commands::mcp_cmd::toggle_mcp_server_by_client(
        args.client,
        args.name,
        args.disabled,
    )
    .await
    .map_err(|error| ai_error("切换 MCP 服务器状态失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_delete_mcp_server(args: AiMcpServerArgs) -> Result<(), IpcError> {
    crate::ai::backend::commands::mcp_cmd::delete_mcp_server_by_client(args.client, args.name)
        .await
        .map_err(|error| ai_error("删除 MCP 服务器失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_copy_mcp_server(args: AiCopyMcpServerArgs) -> Result<(), IpcError> {
    crate::ai::backend::commands::mcp_cmd::copy_mcp_server_to_client(
        args.from_client,
        args.to_client,
        args.name,
        args.overwrite,
    )
    .await
    .map_err(|error| ai_error("复制 MCP 服务器失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_mcp_oauth_status(args: AiMcpServerArgs) -> Result<AiMcpOAuthStatus, IpcError> {
    crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_status_for_client(args.client, args.name)
        .await
        .map(AiMcpOAuthStatus::from)
        .map_err(|error| ai_error("读取 MCP OAuth 状态失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_mcp_oauth_authorize(args: AiMcpServerArgs) -> Result<(), IpcError> {
    crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_authorize_for_client(
        args.client,
        args.name,
    )
    .await
    .map_err(|error| ai_error("MCP OAuth 授权失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_mcp_oauth_cancel(args: AiMcpServerArgs) -> Result<(), IpcError> {
    crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_cancel_authorize_for_client(
        args.client,
        args.name,
    )
    .await
    .map_err(|error| ai_error("取消 MCP OAuth 授权失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_mcp_oauth_refresh(args: AiMcpServerArgs) -> Result<AiMcpOAuthStatus, IpcError> {
    crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_refresh_for_client(
        args.client,
        args.name,
    )
    .await
    .map(AiMcpOAuthStatus::from)
    .map_err(|error| ai_error("刷新 MCP OAuth Token 失败", error))
}

#[tauri::command]
#[specta::specta]
pub async fn ai_mcp_oauth_revoke(args: AiMcpServerArgs) -> Result<String, IpcError> {
    crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_revoke_for_client(args.client, args.name)
        .await
        .map_err(|error| ai_error("撤销 MCP OAuth 授权失败", error))
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionArgs {
    pub workspace_hash: String,
    pub session_id: String,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiLoadSessionArgs {
    pub workspace_hash: String,
    pub session_id: String,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkspaceArgs {
    pub workspace_hash: String,
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiExportSessionArgs {
    pub workspace_hash: String,
    pub session_id: String,
    pub format: String,
    pub path: String,
}

#[tauri::command]
#[specta::specta]
pub async fn ai_list_session_tree(state: State<'_, AiState>) -> Result<AiSessionTree, IpcError> {
    let storage = state.sessions.clone();
    run_session_task("读取 AI 会话列表失败", move || {
        storage.list_session_tree().map(AiSessionTree::from)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_load_session(
    state: State<'_, AiState>,
    args: AiLoadSessionArgs,
) -> Result<AiSessionPage, IpcError> {
    let storage = state.sessions.clone();
    run_session_task("加载 AI 会话失败", move || {
        storage
            .load_session_page(
                &args.workspace_hash,
                &args.session_id,
                args.page,
                args.page_size,
            )
            .map(AiSessionPage::from)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_delete_session(
    state: State<'_, AiState>,
    args: AiSessionArgs,
) -> Result<(), IpcError> {
    let storage = state.sessions.clone();
    run_session_task("删除 AI 会话失败", move || {
        storage.delete_session(&args.workspace_hash, &args.session_id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_delete_workspace(
    state: State<'_, AiState>,
    args: AiWorkspaceArgs,
) -> Result<(), IpcError> {
    let storage = state.sessions.clone();
    run_session_task("删除 AI 会话工作区失败", move || {
        storage.delete_workspace(&args.workspace_hash)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_export_session(
    state: State<'_, AiState>,
    args: AiExportSessionArgs,
) -> Result<String, IpcError> {
    let storage = state.sessions.clone();
    run_session_task("导出 AI 会话失败", move || {
        let format = match args.format.as_str() {
            "json" => crate::ai::backend::services::session_storage::ExportFormat::Json,
            "markdown" => crate::ai::backend::services::session_storage::ExportFormat::Markdown,
            value => return Err(anyhow::anyhow!("不支持的导出格式: {value}")),
        };
        let path = PathBuf::from(&args.path);
        storage.export_session_to_path(&args.workspace_hash, &args.session_id, format, &path)?;
        Ok(path.to_string_lossy().to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ai_reveal_session_file(
    app: AppHandle,
    state: State<'_, AiState>,
    args: AiSessionArgs,
) -> Result<String, IpcError> {
    let storage = state.sessions.clone();
    let path = run_session_task("定位 AI 会话文件失败", move || {
        storage.get_session_file_path(&args.workspace_hash, &args.session_id)
    })
    .await?;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|error| ai_error("打开 AI 会话文件位置失败", error))?;
    Ok(path)
}

#[tauri::command]
#[specta::specta]
pub async fn ai_refresh_session_cache() -> Result<(), IpcError> {
    crate::ai::backend::services::external_sessions::invalidate_cache();
    Ok(())
}

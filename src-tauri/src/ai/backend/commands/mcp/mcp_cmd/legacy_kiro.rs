use crate::ai::backend::commands::common::run_blocking_task;
use crate::ai::backend::kiro::settings::mcp::{McpConfig, McpServer};

#[tauri::command]
pub async fn get_mcp_config(project_dir: Option<String>) -> Result<McpConfig, String> {
    run_blocking_task(move || McpConfig::load_merged(project_dir.as_deref())).await
}

#[tauri::command]
pub async fn save_mcp_server(
    name: String,
    config: McpServer,
    project_dir: Option<String>,
) -> Result<(), String> {
    run_blocking_task(move || {
        validate_mcp_server(&config)?;

        if let Some(project_dir) = project_dir {
            let path = McpConfig::project_config_path(&project_dir);
            let mut mcp_config = McpConfig::load_from_path(&path)?;
            mcp_config.mcp_servers.insert(name, config);
            mcp_config.save_to_path(&path)
        } else {
            let mut mcp_config = McpConfig::load()?;
            mcp_config.mcp_servers.insert(name, config);
            mcp_config.save()
        }
    })
    .await
}

fn validate_mcp_server(config: &McpServer) -> Result<(), String> {
    match config {
        McpServer::Command(cmd) => {
            if cmd.command.trim().is_empty() {
                return Err("command 字段不能为空".to_string());
            }
            if cmd.auto_approve.iter().any(|tool| tool.trim().is_empty()) {
                return Err("autoApprove 中不能包含空字符串".to_string());
            }
            Ok(())
        }
        McpServer::Url(url_config) => {
            if url_config.url.trim().is_empty() {
                return Err("url 字段不能为空".to_string());
            }
            if !url_config.url.starts_with("http://") && !url_config.url.starts_with("https://") {
                return Err("url 必须以 http:// 或 https:// 开头".to_string());
            }
            Ok(())
        }
    }
}

#[tauri::command]
pub async fn delete_mcp_server(name: String, project_dir: Option<String>) -> Result<(), String> {
    run_blocking_task(move || {
        if let Some(project_dir) = project_dir {
            let path = McpConfig::project_config_path(&project_dir);
            let mut mcp_config = McpConfig::load_from_path(&path)?;
            mcp_config.mcp_servers.remove(&name);
            mcp_config.save_to_path(&path)
        } else {
            let mut mcp_config = McpConfig::load()?;
            mcp_config.mcp_servers.remove(&name);
            mcp_config.save()
        }
    })
    .await
}

#[tauri::command]
pub async fn toggle_mcp_server(
    name: String,
    disabled: bool,
    project_dir: Option<String>,
) -> Result<(), String> {
    run_blocking_task(move || {
        let mut config = if let Some(project_dir) = project_dir.as_deref() {
            McpConfig::load_from_path(&McpConfig::project_config_path(project_dir))?
        } else {
            McpConfig::load()?
        };

        let server = config
            .mcp_servers
            .get_mut(&name)
            .ok_or_else(|| format!("服务器 {name} 不存在"))?;

        match server {
            McpServer::Command(cmd) => cmd.disabled = disabled,
            McpServer::Url(url) => url.disabled = disabled,
        }

        if let Some(project_dir) = project_dir {
            config.save_to_path(&McpConfig::project_config_path(&project_dir))
        } else {
            config.save()
        }
    })
    .await
}

#[tauri::command]
pub async fn get_mcp_tool_stats(project_dir: Option<String>) -> Result<serde_json::Value, String> {
    run_blocking_task(move || {
        let mcp_config = McpConfig::load_merged(project_dir.as_deref())?;
        let total_servers = mcp_config.mcp_servers.len();
        let enabled_servers = mcp_config
            .mcp_servers
            .values()
            .filter(|server| match server {
                McpServer::Command(cmd) => !cmd.disabled,
                McpServer::Url(url) => !url.disabled,
            })
            .count();

        Ok(serde_json::json!({
            "totalServers": total_servers,
            "enabledServers": enabled_servers,
        }))
    })
    .await
}

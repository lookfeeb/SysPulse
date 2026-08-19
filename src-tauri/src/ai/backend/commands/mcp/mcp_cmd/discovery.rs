use super::adapters::{read_mcp_server_for_client, restore_mcp_server_for_client, McpClientKind};
use crate::ai::backend::commands::common::run_blocking_task;
use crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_revoke_for_client;
use crate::ai::backend::kiro::settings::mcp::McpConfig;
use crate::ai::backend::utils::fs::atomic_write;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[tauri::command]
pub async fn discover_and_import_mcp_servers(client: String) -> Result<Vec<String>, String> {
    run_blocking_task(move || {
        let client = McpClientKind::parse(&client)?.as_key().to_string();
        let mut imported: Vec<String> = Vec::new();
        for (name, server) in scan_external_mcp_servers() {
            if read_mcp_server_for_client(&client, &name).is_ok() {
                continue;
            }
            // 统一走配置协调器。None 表示原位置不存在；若并发期间刚被其它
            // 程序创建，安装函数会拒绝覆盖并保留对方配置。
            match super::adapters::install_mcp_server_for_client(&client, &name, server, false) {
                Ok(None) => imported.push(name),
                Ok(Some(previous)) => {
                    restore_mcp_server_for_client(&client, &name, Some(previous))?;
                }
                Err(error) if error.contains("已存在 MCP 服务器") => {}
                Err(error) => return Err(format!("导入 {name} 到 {client} 失败: {error}")),
            }
        }
        imported.sort();
        Ok(imported)
    })
    .await
}

fn external_mcp_config_files() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_mcp_files(&home.join(".codex"), 6, &mut files);
    files
}

fn collect_mcp_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_mcp_files(&path, depth - 1, out);
        } else if matches!(
            path.file_name().and_then(|n| n.to_str()),
            Some("mcp.json" | ".mcp.json")
        ) {
            out.push(path);
        }
    }
}

fn scan_external_mcp_servers() -> HashMap<String, serde_json::Value> {
    let mut result = HashMap::new();
    for path in external_mcp_config_files() {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(servers) = value.get("mcpServers").and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, raw) in servers {
            if raw.is_object() {
                result.entry(name.clone()).or_insert_with(|| raw.clone());
            }
        }
    }
    result
}

#[tauri::command]
pub async fn delete_mcp_server_synced(name: String) -> Result<(), String> {
    let _ = mcp_oauth_revoke_for_client("kiro".to_string(), name.clone()).await?;
    run_blocking_task(move || {
        let mut config = McpConfig::load()?;
        config.mcp_servers.remove(&name);
        config.save()?;

        let name_ref: &str = &name;
        std::thread::scope(|scope| {
            for path in external_mcp_config_files() {
                scope.spawn(move || {
                    if let Err(err) = remove_server_from_file(&path, name_ref) {
                        log::warn!("同步删除外部 MCP 配置失败 ({}): {err}", path.display());
                    }
                });
            }
        });
        Ok(())
    })
    .await
}

fn remove_server_from_file(path: &Path, name: &str) -> Result<(), String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(());
    };
    let removed = value
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
        .is_some_and(|m| m.remove(name).is_some());
    if removed {
        let content = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("序列化外部 MCP 配置失败: {e}"))?;
        atomic_write(path, &content, "外部 MCP 配置")?;
    }
    Ok(())
}

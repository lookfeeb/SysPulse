use super::adapters::{read_mcp_server_for_client, restore_mcp_server_for_client, McpClientKind};
use crate::ai::backend::commands::common::run_blocking_task;
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

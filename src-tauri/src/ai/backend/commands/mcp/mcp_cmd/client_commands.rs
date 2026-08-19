use crate::ai::backend::commands::common::run_blocking_task;
use crate::ai::backend::commands::mcp_oauth_cmd::mcp_oauth_revoke_for_client;
use crate::ai::backend::oauth_store::{
    get_mcp_oauth_binding, get_mcp_oauth_store, get_or_init_proxy_runtime, proxy_url_for_binding,
    replace_mcp_oauth_binding, unbind_mcp_oauth_server,
};

use super::adapters::{
    delete_mcp_server_for_client, install_mcp_server_for_client, load_mcp_items_for_client,
    read_mcp_server_for_client, restore_mcp_server_for_client, set_mcp_server_disabled_for_client,
    McpClientKind,
};
use super::types::{McpClientStats, McpClientsOverview, McpServerItem};

#[tauri::command]
pub async fn get_mcp_config_by_client(client: String) -> Result<Vec<McpServerItem>, String> {
    run_blocking_task(move || load_mcp_items_for_client(McpClientKind::parse(&client)?)).await
}

fn stats_for_items(client: &str, items: &[McpServerItem]) -> McpClientStats {
    let enabled_servers = items.iter().filter(|server| !server.disabled).count();
    McpClientStats {
        client: client.to_string(),
        total_servers: items.len(),
        enabled_servers,
    }
}

fn server_requires_oauth_revoke(server_type: &str) -> bool {
    matches!(server_type, "url" | "http" | "sse")
}

#[tauri::command]
pub async fn get_mcp_tool_stats_by_client(client: String) -> Result<McpClientStats, String> {
    run_blocking_task(move || {
        let kind = McpClientKind::parse(&client)?;
        let items = load_mcp_items_for_client(kind)?;
        Ok(stats_for_items(kind.as_key(), &items))
    })
    .await
}

#[tauri::command]
pub async fn get_mcp_clients_overview() -> Result<McpClientsOverview, String> {
    run_blocking_task(|| {
        let mut clients = Vec::new();
        for kind in [
            McpClientKind::Kiro,
            McpClientKind::Codex,
            McpClientKind::ClaudeCli,
        ] {
            let items = load_mcp_items_for_client(kind)?;
            clients.push(stats_for_items(kind.as_key(), &items));
        }

        Ok(McpClientsOverview {
            total_servers: clients.iter().map(|s| s.total_servers).sum(),
            enabled_servers: clients.iter().map(|s| s.enabled_servers).sum(),
            clients,
        })
    })
    .await
}

#[tauri::command]
pub async fn toggle_mcp_server_by_client(
    client: String,
    name: String,
    disabled: bool,
) -> Result<(), String> {
    run_blocking_task(move || set_mcp_server_disabled_for_client(&client, &name, disabled)).await
}

#[tauri::command]
pub async fn delete_mcp_server_by_client(client: String, name: String) -> Result<(), String> {
    let server_type = run_blocking_task({
        let client = client.clone();
        let name = name.clone();
        move || Ok(read_mcp_server_for_client(&client, &name)?.server_type)
    })
    .await?;
    if server_requires_oauth_revoke(&server_type) {
        let _ = mcp_oauth_revoke_for_client(client.clone(), name.clone()).await?;
    } else {
        // command 型服务器无需读取 URL；仅清理可能由旧版本遗留的本地绑定。
        let _ = unbind_mcp_oauth_server(&client, &name)?;
    }
    run_blocking_task(move || delete_mcp_server_for_client(&client, &name)).await
}

#[tauri::command]
pub async fn copy_mcp_server_to_client(
    from_client: String,
    to_client: String,
    name: String,
    overwrite: bool,
) -> Result<(), String> {
    if from_client.eq_ignore_ascii_case(&to_client) {
        return Err("源客户端与目标客户端不能相同".to_string());
    }
    let (source, source_binding, source_credential) = run_blocking_task({
        let from_client = from_client.clone();
        let name = name.clone();
        move || {
            let source = read_mcp_server_for_client(&from_client, &name)?;
            let source_binding = get_mcp_oauth_binding(&from_client, &name)?;
            let source_credential = source_binding
                .as_ref()
                .map(|credential_key| {
                    get_mcp_oauth_store()?
                        .creds_by_key
                        .get(credential_key)
                        .cloned()
                        .ok_or_else(|| "源 MCP 的 OAuth 绑定对应凭据已丢失".to_string())
                })
                .transpose()?;
            Ok((source, source_binding, source_credential))
        }
    })
    .await?;

    let mut target_raw = source.raw;
    if let (Some(credential_key), Some(credential)) =
        (source_binding.as_deref(), source_credential.as_ref())
    {
        let (port, secret) = get_or_init_proxy_runtime()?;
        let proxy = proxy_url_for_binding(
            port,
            &secret,
            credential_key,
            &credential.mcp_endpoint,
            &name,
        );
        let object = target_raw
            .as_object_mut()
            .ok_or_else(|| "源 MCP 配置不是对象".to_string())?;
        object.insert("url".to_string(), serde_json::Value::String(proxy));
    }

    let previous = run_blocking_task({
        let to_client = to_client.clone();
        let name = name.clone();
        move || install_mcp_server_for_client(&to_client, &name, target_raw, overwrite)
    })
    .await?;

    let displaced = match replace_mcp_oauth_binding(&to_client, &name, source_binding.as_deref()) {
        Ok(displaced) => displaced,
        Err(error) => {
            let rollback = run_blocking_task({
                let to_client = to_client.clone();
                let name = name.clone();
                move || restore_mcp_server_for_client(&to_client, &name, previous)
            })
            .await;
            return Err(match rollback {
                Ok(()) => format!("复制绑定提交失败，配置已回滚: {error}"),
                Err(rollback_error) => {
                    format!("复制绑定提交失败: {error}；配置回滚也失败: {rollback_error}")
                }
            });
        }
    };

    if let Some((_, credential)) = displaced {
        if let Err(error) = super::super::mcp_oauth_cmd::revoke_remote_credential(&credential).await
        {
            log::warn!("复制 MCP 后清理目标旧 OAuth 授权失败: {error}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::server_requires_oauth_revoke;

    #[test]
    fn command_server_delete_skips_oauth_revoke_path() {
        assert!(!server_requires_oauth_revoke("command"));
        assert!(server_requires_oauth_revoke("url"));
        assert!(server_requires_oauth_revoke("http"));
        assert!(server_requires_oauth_revoke("sse"));
    }
}

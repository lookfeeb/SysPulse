use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::ai::backend::runtime::data_dir;

const DPAPI_PREFIX: &str = "dpapi:v1:";

fn oauth_cache() -> &'static super::oauth_cache::OAuthCache {
    static CACHE: OnceLock<super::oauth_cache::OAuthCache> = OnceLock::new();
    CACHE.get_or_init(super::oauth_cache::OAuthCache::default)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthCred {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    /// 最近一次刷新尝试的 Unix 时间；用于 expires_in 缺失时的保守定时续期。
    #[serde(default)]
    pub last_refresh_attempt_at: i64,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    /// RFC 7009 revocation endpoint. Older credentials may not have this
    /// value; in that case revocation falls back to local cleanup only.
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    pub mcp_endpoint: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStore {
    #[serde(default)]
    pub creds: HashMap<String, McpOAuthCred>,
    #[serde(default)]
    pub creds_by_key: HashMap<String, McpOAuthCred>,
    #[serde(default)]
    pub server_bindings: HashMap<String, String>,
    #[serde(default)]
    pub refresh_failures: HashMap<String, String>,
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub proxy_secret: Option<String>,
}

mod persistence;
mod proxy;
pub use persistence::get_mcp_oauth_store;
use persistence::*;
pub use proxy::*;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub enum McpOAuthConditionalUpdate {
    Applied(McpOAuthCred),
    Changed(McpOAuthCred),
    Missing,
}

#[derive(Clone)]
pub enum McpOAuthCredentialRollback {
    Restored(Box<McpOAuthCred>),
    Changed,
    Missing,
    InUse,
}

fn credential_matches(
    cred: &McpOAuthCred,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
) -> bool {
    cred.access_token == expected_access_token
        && cred.refresh_token.as_deref() == expected_refresh_token
}

pub fn upsert_mcp_oauth_cred(credential_key: &str, cred: McpOAuthCred) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    store.creds_by_key.insert(credential_key.to_string(), cred);
    store.refresh_failures.remove(credential_key);
    save_mcp_oauth_store_unlocked(&store)
}

fn rollback_credential_in_store_if_current(
    store: &mut McpOAuthStore,
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    previous: Option<McpOAuthCred>,
    previous_refresh_failure: Option<String>,
) -> McpOAuthCredentialRollback {
    let Some(current) = store.creds_by_key.get(credential_key).cloned() else {
        return McpOAuthCredentialRollback::Missing;
    };
    if !credential_matches(&current, expected_access_token, expected_refresh_token) {
        return McpOAuthCredentialRollback::Changed;
    }
    if previous.is_none()
        && store
            .server_bindings
            .values()
            .any(|value| value == credential_key)
    {
        return McpOAuthCredentialRollback::InUse;
    }

    match previous {
        Some(previous) => {
            store
                .creds_by_key
                .insert(credential_key.to_string(), previous);
        }
        None => {
            store.creds_by_key.remove(credential_key);
        }
    }
    match previous_refresh_failure {
        Some(message) => {
            store
                .refresh_failures
                .insert(credential_key.to_string(), message);
        }
        None => {
            store.refresh_failures.remove(credential_key);
        }
    }
    McpOAuthCredentialRollback::Restored(Box::new(current))
}

/// 授权提交失败时，仅在当前值仍是本次新签发 Token 的前提下恢复旧快照。
/// 返回被替换或删除的新凭据，供调用方异步撤销，避免覆盖并发刷新结果。
pub fn rollback_mcp_oauth_cred_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    previous: Option<McpOAuthCred>,
    previous_refresh_failure: Option<String>,
) -> Result<McpOAuthCredentialRollback, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let result = rollback_credential_in_store_if_current(
        &mut store,
        credential_key,
        expected_access_token,
        expected_refresh_token,
        previous,
        previous_refresh_failure,
    );
    if matches!(result, McpOAuthCredentialRollback::Restored(_)) {
        save_mcp_oauth_store_unlocked(&store)?;
    }
    Ok(result)
}

pub fn upsert_mcp_oauth_cred_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    updated: McpOAuthCred,
) -> Result<McpOAuthConditionalUpdate, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let current = store.creds_by_key.get(credential_key).cloned();
    match current {
        Some(current)
            if credential_matches(&current, expected_access_token, expected_refresh_token) =>
        {
            store
                .creds_by_key
                .insert(credential_key.to_string(), updated.clone());
            store.refresh_failures.remove(credential_key);
            save_mcp_oauth_store_unlocked(&store)?;
            Ok(McpOAuthConditionalUpdate::Applied(updated))
        }
        Some(current) => Ok(McpOAuthConditionalUpdate::Changed(current)),
        None => Ok(McpOAuthConditionalUpdate::Missing),
    }
}

pub fn set_mcp_oauth_refresh_failure_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    message: String,
) -> Result<McpOAuthConditionalUpdate, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let current = store.creds_by_key.get(credential_key).cloned();
    match current {
        Some(current)
            if credential_matches(&current, expected_access_token, expected_refresh_token) =>
        {
            let mut current = current;
            current.last_refresh_attempt_at = now_secs();
            store
                .creds_by_key
                .insert(credential_key.to_string(), current.clone());
            store
                .refresh_failures
                .insert(credential_key.to_string(), message);
            save_mcp_oauth_store_unlocked(&store)?;
            Ok(McpOAuthConditionalUpdate::Applied(current))
        }
        Some(current) => Ok(McpOAuthConditionalUpdate::Changed(current)),
        None => Ok(McpOAuthConditionalUpdate::Missing),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn bind_mcp_oauth_server(
    client: &str,
    server_name: &str,
    credential_key: &str,
) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    store
        .server_bindings
        .insert(binding_key(client, server_name), credential_key.to_string());
    save_mcp_oauth_store_unlocked(&store)
}

/// 原子替换（或清除）一个客户端绑定，并回收不再被任何客户端引用的旧凭据。
/// 返回被回收的旧凭据，供调用方在本地提交成功后尽力通知远端撤销。
pub fn replace_mcp_oauth_binding(
    client: &str,
    server_name: &str,
    credential_key: Option<&str>,
) -> Result<Option<(String, McpOAuthCred)>, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    if let Some(credential_key) = credential_key {
        if !store.creds_by_key.contains_key(credential_key) {
            return Err("要绑定的 MCP OAuth 凭据不存在".to_string());
        }
    }

    let binding = binding_key(client, server_name);
    let previous = store.server_bindings.get(&binding).cloned();
    if previous.as_deref() == credential_key {
        return Ok(None);
    }

    match credential_key {
        Some(credential_key) => {
            store
                .server_bindings
                .insert(binding, credential_key.to_string());
        }
        None => {
            store.server_bindings.remove(&binding);
        }
    }

    let removed = previous.and_then(|previous_key| {
        let still_used = store
            .server_bindings
            .values()
            .any(|value| value == &previous_key);
        if still_used {
            return None;
        }
        store.refresh_failures.remove(&previous_key);
        store
            .creds_by_key
            .remove(&previous_key)
            .map(|credential| (previous_key, credential))
    });
    save_mcp_oauth_store_unlocked(&store)?;
    Ok(removed)
}

pub fn get_mcp_oauth_binding(client: &str, server_name: &str) -> Result<Option<String>, String> {
    let store = get_mcp_oauth_store()?;
    Ok(store
        .server_bindings
        .get(&binding_key(client, server_name))
        .cloned())
}

/// 读取绑定及其共享状态，用于在服务端撤销前判断该凭据是否仍被其他
/// 客户端使用。返回值为 `(credential_key, credential, removed_last)`。
pub fn get_mcp_oauth_binding_details(
    client: &str,
    server_name: &str,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    let store = get_mcp_oauth_store()?;
    let binding = binding_key(client, server_name);
    let Some(credential_key) = store.server_bindings.get(&binding).cloned() else {
        return Ok(None);
    };
    let Some(cred) = store.creds_by_key.get(&credential_key).cloned() else {
        return Ok(None);
    };
    let still_used = store
        .server_bindings
        .iter()
        .any(|(key, value)| key != &binding && value == &credential_key);
    Ok(Some((credential_key, cred, !still_used)))
}

pub fn unbind_mcp_oauth_server(
    client: &str,
    server_name: &str,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    unbind_mcp_oauth_server_if_current(client, server_name, None)
}

/// 与预读绑定组合成条件删除，避免撤销请求等待网络期间其它客户端
/// 刚好改绑后误删/误撤销新的凭据。
pub fn unbind_mcp_oauth_server_if_current(
    client: &str,
    server_name: &str,
    expected_credential_key: Option<&str>,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let binding = binding_key(client, server_name);
    let Some(credential_key) = store.server_bindings.get(&binding).cloned() else {
        save_mcp_oauth_store_unlocked(&store)?;
        return Ok(None);
    };
    if expected_credential_key.is_some_and(|expected| expected != credential_key) {
        return Err("MCP OAuth 绑定在撤销期间发生变化，请重新操作".to_string());
    }
    store.server_bindings.remove(&binding);

    let still_used = store.server_bindings.values().any(|v| v == &credential_key);
    let cred = store.creds_by_key.get(&credential_key).cloned();
    let removed_last = !still_used;

    if removed_last {
        store.creds_by_key.remove(&credential_key);
        store.refresh_failures.remove(&credential_key);
    }

    save_mcp_oauth_store_unlocked(&store)?;
    Ok(cred.map(|c| (credential_key, c, removed_last)))
}

pub fn get_or_init_proxy_runtime() -> Result<(u16, String), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let mut changed = false;

    if store.proxy_port.is_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("分配反代端口失败: {e}"))?;
        store.proxy_port = Some(
            listener
                .local_addr()
                .map_err(|e| format!("获取端口失败: {e}"))?
                .port(),
        );
        changed = true;
    }

    if store.proxy_secret.is_none() {
        store.proxy_secret = Some(uuid::Uuid::new_v4().simple().to_string());
        changed = true;
    }

    if changed {
        save_mcp_oauth_store_unlocked(&store)?;
    }

    let port = store
        .proxy_port
        .ok_or_else(|| "MCP OAuth 反代端口初始化失败".to_string())?;
    let secret = store
        .proxy_secret
        .ok_or_else(|| "MCP OAuth 反代密钥初始化失败".to_string())?;

    Ok((port, secret))
}

pub fn set_proxy_runtime_port(port: u16) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    if store.proxy_port != Some(port) {
        store.proxy_port = Some(port);
        save_mcp_oauth_store_unlocked(&store)?;
    }
    Ok(())
}

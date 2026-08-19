// 远程 MCP OAuth 令牌后台自动刷新任务
// 每 60s 扫描凭证，对 10 分钟内过期者用 refresh_token 续期，处理 refresh_token 轮换并持久化

use crate::ai::backend::mcp_oauth::refresh_stored_credential;
use crate::ai::backend::oauth_store::{
    get_mcp_oauth_store, mcp_oauth_failure_needs_reauth, McpOAuthCred,
};
use futures_util::stream::{self, StreamExt};
use tauri::{AppHandle, Emitter};
use tokio::time::{interval, Duration};

const LOOP_INTERVAL_SECONDS: u64 = 60;
const REFRESH_THRESHOLD_SECONDS: i64 = 600; // 提前 10 分钟刷新
const UNKNOWN_EXPIRY_REFRESH_INTERVAL_SECONDS: i64 = 15 * 60;
const MAX_CONCURRENT_REFRESHES: usize = 4;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn credential_refresh_due(cred: &McpOAuthCred, now: i64) -> bool {
    if cred.refresh_token.is_none() {
        return false;
    }
    if cred.expires_at > 0 {
        return cred.expires_at - now < REFRESH_THRESHOLD_SECONDS;
    }
    // 某些 OAuth 服务不返回 expires_in。不能因此永久失去自动续期，
    // 也不能每 60 秒反复消费轮换 refresh token；采用保守的 15 分钟探测。
    cred.last_refresh_attempt_at <= 0
        || now.saturating_sub(cred.last_refresh_attempt_at)
            >= UNKNOWN_EXPIRY_REFRESH_INTERVAL_SECONDS
}

async fn refresh_due(app_handle: &AppHandle) -> Result<(), String> {
    let store = match get_mcp_oauth_store() {
        Ok(s) => s,
        Err(error) => {
            log::error!("读取 MCP OAuth 凭据失败，已跳过本轮自动续期: {error}");
            return Ok(());
        }
    };
    let now = now_secs();
    let due = store
        .creds_by_key
        .iter()
        .filter(|(key, credential)| {
            !store
                .refresh_failures
                .get(*key)
                .is_some_and(|message| mcp_oauth_failure_needs_reauth(message))
                && credential_refresh_due(credential, now)
        })
        .map(|(key, credential)| (key.clone(), credential.clone()))
        .collect::<Vec<_>>();

    let results = stream::iter(due)
        .map(|(key, credential)| async move {
            let result = refresh_stored_credential(&key, Some(&credential)).await;
            (key, result)
        })
        .buffer_unordered(MAX_CONCURRENT_REFRESHES)
        .collect::<Vec<_>>()
        .await;
    let changed = !results.is_empty();

    for (key, result) in results {
        match result {
            Ok(_updated) => {
                log::info!("MCP token refreshed: {key}");
            }
            Err(error) => {
                let msg = error.to_string();
                if !mcp_oauth_failure_needs_reauth(&msg) {
                    log::error!("MCP token refresh failed [{key}]: {msg}");
                }
            }
        }
    }

    if changed {
        let _ = app_handle.emit("mcp-tokens-updated", ());
    }
    Ok(())
}

/// 启动 MCP 令牌刷新循环（供 main.rs 调用）
pub fn start_mcp_token_refresh_loop(app_handle: AppHandle) {
    log::info!("Starting MCP token refresh background task");
    tauri::async_runtime::spawn(async move {
        let mut timer = interval(Duration::from_secs(LOOP_INTERVAL_SECONDS));
        loop {
            timer.tick().await;
            if let Err(e) = refresh_due(&app_handle).await {
                log::error!("MCP token refresh loop error: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{credential_refresh_due, UNKNOWN_EXPIRY_REFRESH_INTERVAL_SECONDS};
    use crate::ai::backend::oauth_store::McpOAuthCred;

    fn credential(expires_at: i64, last_refresh_attempt_at: i64) -> McpOAuthCred {
        McpOAuthCred {
            client_id: "client".to_string(),
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at,
            last_refresh_attempt_at,
            auth_endpoint: "https://example.com/authorize".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            revocation_endpoint: None,
            mcp_endpoint: "https://example.com/mcp".to_string(),
            resource: "https://example.com/mcp".to_string(),
        }
    }

    #[test]
    fn refreshes_known_expiry_inside_threshold() {
        assert!(credential_refresh_due(&credential(1_500, 0), 1_000));
        assert!(!credential_refresh_due(&credential(2_000, 0), 1_000));
    }

    #[test]
    fn refreshes_unknown_expiry_on_conservative_interval() {
        assert!(!credential_refresh_due(
            &credential(0, 1_000),
            1_000 + UNKNOWN_EXPIRY_REFRESH_INTERVAL_SECONDS - 1
        ));
        assert!(credential_refresh_due(
            &credential(0, 1_000),
            1_000 + UNKNOWN_EXPIRY_REFRESH_INTERVAL_SECONDS
        ));
        assert!(!credential_refresh_due(&credential(0, 1_000), 999));
    }
}

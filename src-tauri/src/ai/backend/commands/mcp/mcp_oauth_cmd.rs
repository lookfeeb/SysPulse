// 远程 MCP 服务器 OAuth 命令
// 多客户端统一走共享 credentialKey，并把 URL 改写为本地反代地址。

#![allow(clippy::needless_pass_by_value)]

use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use crate::ai::backend::commands::common::run_blocking_task;
use crate::ai::backend::commands::mcp_cmd::{
    load_mcp_items_for_client, read_mcp_server_url_for_client,
    write_mcp_server_url_for_client_if_current, McpClientKind,
};
use crate::ai::backend::mcp_oauth::{
    discard_authorize_outcome, discard_credential_tokens, discover_endpoints,
    discover_protected_resource_with_cancel, lock_credential_operation, probe_oauth_endpoints,
    refresh_stored_credential, revoke_token, run_authorize, OAuthProbeResult,
};
use crate::ai::backend::oauth_store::{
    bind_mcp_oauth_server, binding_key, canonical_mcp_endpoint, credential_key_matches_endpoint,
    get_mcp_oauth_binding, get_mcp_oauth_binding_details, get_mcp_oauth_store,
    get_or_init_proxy_runtime, is_mcp_oauth_proxy_url, mcp_oauth_failure_needs_reauth,
    normalized_credential_key, parse_mcp_oauth_proxy_url, proxy_url_for_binding,
    recover_proxy_credential_key, replace_mcp_oauth_binding, rollback_mcp_oauth_cred_if_current,
    unbind_mcp_oauth_server, unbind_mcp_oauth_server_if_current, upsert_mcp_oauth_cred,
    McpOAuthCred, McpOAuthCredentialRollback, McpOAuthStore,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStatus {
    pub oauth_supported: Option<bool>,
    pub authorized: bool,
    pub expires_at: i64,
    pub expiring_soon: bool,
    pub expired: bool,
    pub refresh_failed: bool,
    pub needs_reauth: bool,
    pub credential_key: Option<String>,
    pub message: Option<String>,
}

fn pending_authorizations() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_pending_authorization(key: &str) -> Result<Arc<AtomicBool>, String> {
    let mut pending = pending_authorizations()
        .lock()
        .map_err(|_| "MCP OAuth 授权状态锁已损坏".to_string())?;
    if pending.contains_key(key) {
        return Err("该 MCP 服务器正在授权中".to_string());
    }

    let cancelled = Arc::new(AtomicBool::new(false));
    pending.insert(key.to_string(), cancelled.clone());
    Ok(cancelled)
}

fn remove_pending_authorization(key: &str) {
    if let Ok(mut pending) = pending_authorizations().lock() {
        pending.remove(key);
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn refresh_credential(credential_key: &str) -> Result<McpOAuthCred, String> {
    refresh_stored_credential(credential_key, None).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteRevokeResult {
    Revoked,
    Unsupported,
}

/// 尝试按 RFC 7009 通知授权服务器撤销 token。
/// 旧凭据可能没有保存 revocation_endpoint；此时重新发现一次，发现不到
/// 就明确返回 Unsupported，由上层只做本机清理并提示用户。
pub(crate) async fn revoke_remote_credential(
    cred: &McpOAuthCred,
) -> Result<RemoteRevokeResult, String> {
    let endpoint = if let Some(endpoint) = cred.revocation_endpoint.clone() {
        Some(endpoint)
    } else {
        match tokio::time::timeout(
            Duration::from_secs(8),
            discover_endpoints(&cred.mcp_endpoint),
        )
        .await
        {
            Ok(Ok(endpoints)) => endpoints.revocation_endpoint,
            Ok(Err(error)) => {
                log::warn!(
                    "OAuth 撤销时重新发现授权元数据失败 ({}): {}",
                    cred.mcp_endpoint,
                    error
                );
                None
            }
            Err(_) => {
                log::warn!("OAuth 撤销时重新发现授权元数据超时: {}", cred.mcp_endpoint);
                None
            }
        }
    };
    let Some(endpoint) = endpoint else {
        return Ok(RemoteRevokeResult::Unsupported);
    };

    let mut errors = Vec::new();
    if let Some(refresh_token) = cred.refresh_token.as_deref() {
        match tokio::time::timeout(
            Duration::from_secs(8),
            revoke_token(
                &endpoint,
                &cred.client_id,
                refresh_token,
                Some("refresh_token"),
            ),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(format!("refresh_token: {error}")),
            Err(_) => errors.push("refresh_token: 请求超时".to_string()),
        }
    }
    if cred
        .refresh_token
        .as_deref()
        .map_or(true, |refresh_token| refresh_token != cred.access_token)
    {
        match tokio::time::timeout(
            Duration::from_secs(8),
            revoke_token(
                &endpoint,
                &cred.client_id,
                &cred.access_token,
                Some("access_token"),
            ),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(format!("access_token: {error}")),
            Err(_) => errors.push("access_token: 请求超时".to_string()),
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok(RemoteRevokeResult::Revoked)
}

#[derive(Debug, Clone)]
struct ResolvedEndpoint {
    endpoint: String,
    credential_key: Option<String>,
    recovered_from_proxy: bool,
    source_url: String,
}

#[derive(Default)]
struct ProxyRecoveryHints {
    endpoints: BTreeSet<String>,
    origins: BTreeSet<String>,
}

fn validated_endpoint(value: &str) -> Option<String> {
    let canonical = canonical_mcp_endpoint(value);
    let url = url::Url::parse(&canonical).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    if url.fragment().is_some() {
        return None;
    }
    Some(canonical)
}

fn add_recovery_hint(hints: &mut ProxyRecoveryHints, value: &str, server_name: &str) {
    if let Some(proxy) = parse_mcp_oauth_proxy_url(value) {
        if proxy.server_name != server_name {
            return;
        }
        if let Some(endpoint) = proxy.mcp_endpoint.as_deref().and_then(validated_endpoint) {
            hints.endpoints.insert(endpoint);
        } else if let Some(origin) = proxy.credential_key {
            hints.origins.insert(origin);
        }
        return;
    }

    if let Some(endpoint) = validated_endpoint(value) {
        hints.endpoints.insert(endpoint);
    }
}

fn recovery_probe_url(origin: &str, path_hint: &str) -> Result<String, String> {
    let mut url =
        url::Url::parse(origin).map_err(|error| format!("旧代理中的 OAuth 来源无效: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("旧代理中的 OAuth 来源必须是 http/https 地址".to_string());
    }
    url.set_query(None);
    url.set_fragment(None);
    url.set_path("");
    url.path_segments_mut()
        .map_err(|_| "旧代理中的 OAuth 来源不能作为基础地址".to_string())?
        .push(path_hint);
    Ok(url.to_string())
}

fn peer_server_urls(client: &str, server_name: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for kind in [
        McpClientKind::Kiro,
        McpClientKind::Codex,
        McpClientKind::ClaudeCli,
    ] {
        if kind.as_key().eq_ignore_ascii_case(client) {
            continue;
        }
        let Ok(items) = load_mcp_items_for_client(kind) else {
            continue;
        };
        if let Some(item) = items.into_iter().find(|item| {
            item.name == server_name && matches!(item.server_type.as_str(), "url" | "http" | "sse")
        }) {
            urls.push(item.detail);
        }
    }
    urls
}

async fn recover_proxy_endpoint(
    current_url: &str,
    server_name: &str,
    peer_urls: &[String],
    cancelled: Option<&AtomicBool>,
) -> Result<String, String> {
    let current_proxy = parse_mcp_oauth_proxy_url(current_url)
        .filter(|proxy| proxy.server_name == server_name)
        .ok_or_else(|| "当前地址不是该服务器的 SysPulse OAuth 代理 URL".to_string())?;
    let current_origin = current_proxy.credential_key.clone();

    let mut hints = ProxyRecoveryHints::default();
    if let Some(endpoint) = current_proxy
        .mcp_endpoint
        .as_deref()
        .and_then(validated_endpoint)
    {
        hints.endpoints.insert(endpoint);
    } else if let Some(origin) = current_proxy.credential_key {
        hints.origins.insert(origin);
    }
    for peer_url in peer_urls {
        add_recovery_hint(&mut hints, peer_url, server_name);
    }

    if let Some(current_origin) = current_origin {
        hints
            .endpoints
            .retain(|endpoint| credential_key_matches_endpoint(&current_origin, endpoint));
        hints.origins.clear();
        hints.origins.insert(current_origin);
    }

    if hints.endpoints.len() == 1 {
        return Ok(hints.endpoints.into_iter().next().unwrap());
    }
    if hints.endpoints.len() > 1 {
        return Err(
            "其他客户端存在多个不同的同名 MCP 真实端点，已停止自动恢复以避免写错配置".to_string(),
        );
    }

    let discoveries = futures_util::future::join_all(hints.origins.into_iter().map(|origin| {
        let server_name = server_name.to_string();
        async move {
            if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                return Err("授权已取消".to_string());
            }
            let standard_mcp_url = recovery_probe_url(&origin, "mcp")?;
            // 标准 `/mcp` 优先。部分服务（Notion）会对任意 well-known
            // 路径回显 resource，先探测 `/{server_name}` 会把名称误判成端点。
            let mut probe_urls = vec![standard_mcp_url];
            let named_url = recovery_probe_url(&origin, &server_name)?;
            if !probe_urls.contains(&named_url) {
                probe_urls.push(named_url);
            }
            let mut failures = Vec::new();
            for probe_url in probe_urls {
                match tokio::time::timeout(
                    Duration::from_secs(8),
                    discover_protected_resource_with_cancel(&probe_url, cancelled),
                )
                .await
                {
                    Ok(Ok(Some(endpoint))) => return Ok((origin, Some(endpoint))),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) if error == "授权已取消" => return Err(error),
                    Ok(Err(error)) => failures.push(error),
                    Err(_) => failures.push(format!("{probe_url} 元数据发现超时")),
                }
            }
            if failures.is_empty() {
                Ok((origin, None))
            } else {
                Err(format!("{origin}：{}", failures.join("；")))
            }
        }
    }))
    .await;

    let mut endpoints = BTreeSet::new();
    let mut failures = Vec::new();
    for discovery in discoveries {
        match discovery {
            Ok((_, Some(endpoint))) => {
                if let Some(endpoint) = validated_endpoint(&endpoint) {
                    endpoints.insert(endpoint);
                }
            }
            Ok((origin, None)) => failures.push(format!("{origin} 未提供 RFC 9728 资源元数据")),
            Err(error) => failures.push(error),
        }
    }

    if endpoints.len() == 1 {
        return Ok(endpoints.into_iter().next().unwrap());
    }
    if endpoints.len() > 1 {
        return Err(
            "旧代理来源解析出了多个不同的 MCP 真实端点，已停止自动恢复以避免写错配置".to_string(),
        );
    }

    if failures.is_empty() {
        Err("旧格式代理地址不包含完整端点，且其他客户端没有可用于恢复的唯一同名配置".to_string())
    } else {
        Err(format!(
            "无法自动恢复旧代理的真实端点：{}",
            failures.join("；")
        ))
    }
}

async fn resolve_real_endpoint(
    client: &str,
    server_name: &str,
    current_url: String,
) -> Result<ResolvedEndpoint, String> {
    resolve_real_endpoint_with_cancel(client, server_name, current_url, None).await
}

async fn resolve_real_endpoint_with_cancel(
    client: &str,
    server_name: &str,
    current_url: String,
    cancelled: Option<&AtomicBool>,
) -> Result<ResolvedEndpoint, String> {
    let store = get_mcp_oauth_store()?;
    if let Some(credential_key) = store.server_bindings.get(&binding_key(client, server_name)) {
        if let Some(cred) = store.creds_by_key.get(credential_key) {
            return Ok(ResolvedEndpoint {
                endpoint: cred.mcp_endpoint.clone(),
                credential_key: Some(credential_key.clone()),
                recovered_from_proxy: false,
                source_url: current_url,
            });
        }
    }

    if let Some(credential_key) = recover_proxy_credential_key(&store, &current_url, server_name) {
        if let Some(cred) = store.creds_by_key.get(&credential_key) {
            return Ok(ResolvedEndpoint {
                endpoint: cred.mcp_endpoint.clone(),
                credential_key: Some(credential_key),
                recovered_from_proxy: false,
                source_url: current_url,
            });
        }
    }
    if is_mcp_oauth_proxy_url(&current_url) {
        let peer_urls = run_blocking_task({
            let client = client.to_string();
            let server_name = server_name.to_string();
            move || Ok(peer_server_urls(&client, &server_name))
        })
        .await?;
        let endpoint =
            recover_proxy_endpoint(&current_url, server_name, &peer_urls, cancelled).await?;
        return Ok(ResolvedEndpoint {
            endpoint,
            credential_key: None,
            recovered_from_proxy: true,
            source_url: current_url,
        });
    }
    let endpoint = validated_endpoint(&current_url)
        .ok_or_else(|| "MCP URL 必须是有效的 http/https 地址".to_string())?;
    let needs_writeback = endpoint != current_url;
    Ok(ResolvedEndpoint {
        endpoint,
        credential_key: None,
        recovered_from_proxy: needs_writeback,
        source_url: current_url,
    })
}

async fn persist_recovered_endpoint(
    client: &str,
    server_name: &str,
    resolved: &ResolvedEndpoint,
) -> Result<bool, String> {
    if !resolved.recovered_from_proxy {
        return Ok(false);
    }
    let changed = run_blocking_task({
        let client = client.to_string();
        let server_name = server_name.to_string();
        let source_url = resolved.source_url.clone();
        let endpoint = resolved.endpoint.clone();
        move || {
            write_mcp_server_url_for_client_if_current(
                &client,
                &server_name,
                &source_url,
                &endpoint,
            )
        }
    })
    .await?;
    if changed {
        let _ = unbind_mcp_oauth_server(client, server_name)?;
    }
    Ok(changed)
}

fn can_reuse_client_id(refresh_failure: Option<&str>) -> bool {
    !refresh_failure.is_some_and(|message| {
        let message = message.to_ascii_lowercase();
        message.contains("invalid_client") || message.contains("unauthorized_client")
    })
}

fn authorized_status(store: &McpOAuthStore, credential_key: &str) -> Option<McpOAuthStatus> {
    let cred = store.creds_by_key.get(credential_key)?;
    let now = now_secs();
    let expired = cred.expires_at > 0 && cred.expires_at <= now;
    let refresh_message = store.refresh_failures.get(credential_key).cloned();
    let refresh_failed = refresh_message.is_some();
    let permanent_failure = refresh_message
        .as_deref()
        .is_some_and(mcp_oauth_failure_needs_reauth);
    Some(McpOAuthStatus {
        oauth_supported: Some(true),
        authorized: true,
        expires_at: cred.expires_at,
        expiring_soon: cred.expires_at > 0 && cred.expires_at - now < 600 && !expired,
        expired,
        refresh_failed,
        needs_reauth: permanent_failure || (expired && refresh_failed),
        credential_key: Some(credential_key.to_string()),
        message: refresh_message,
    })
}

fn credentials_share_token(left: &McpOAuthCred, right: &McpOAuthCred) -> bool {
    let left_refresh = left.refresh_token.as_deref();
    let right_refresh = right.refresh_token.as_deref();
    left.access_token == right.access_token
        || left_refresh == Some(right.access_token.as_str())
        || right_refresh == Some(left.access_token.as_str())
        || left_refresh.is_some_and(|token| right_refresh == Some(token))
}

fn rollback_authorized_credential(
    credential_key: &str,
    applied: Option<&McpOAuthCred>,
    previous: Option<&McpOAuthCred>,
    previous_refresh_failure: Option<&str>,
) -> Result<(), String> {
    let Some(applied) = applied else {
        return Ok(());
    };
    match rollback_mcp_oauth_cred_if_current(
        credential_key,
        &applied.access_token,
        applied.refresh_token.as_deref(),
        previous.cloned(),
        previous_refresh_failure.map(str::to_string),
    )? {
        McpOAuthCredentialRollback::Restored(replaced) => {
            if previous.is_some_and(|old| credentials_share_token(&replaced, old)) {
                log::warn!("OAuth 授权回滚后新旧凭据存在重叠 Token，跳过远端撤销");
            } else {
                discard_credential_tokens(*replaced);
            }
            Ok(())
        }
        McpOAuthCredentialRollback::Changed => {
            Err("OAuth 凭据已被其他操作更新，未强制覆盖".to_string())
        }
        McpOAuthCredentialRollback::Missing => Err("OAuth 凭据在回滚前已不存在".to_string()),
        McpOAuthCredentialRollback::InUse => {
            Err("新 OAuth 凭据已被其他客户端绑定，未强制删除".to_string())
        }
    }
}

#[tauri::command]
pub async fn mcp_oauth_authorize_for_client(
    client: String,
    server_name: String,
) -> Result<(), String> {
    let authorization_key = binding_key(&client, &server_name);
    let cancelled = register_pending_authorization(&authorization_key)?;
    let result = mcp_oauth_authorize_for_client_inner(client, server_name, cancelled).await;
    remove_pending_authorization(&authorization_key);
    result
}

async fn mcp_oauth_authorize_for_client_inner(
    client: String,
    server_name: String,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String> {
    let current_url = run_blocking_task({
        let client = client.clone();
        let server_name = server_name.clone();
        move || read_mcp_server_url_for_client(&client, &server_name)
    })
    .await?;
    let resolved = resolve_real_endpoint_with_cancel(
        &client,
        &server_name,
        current_url,
        Some(cancelled.as_ref()),
    )
    .await?;
    if cancelled.load(Ordering::SeqCst) {
        return Err("授权已取消".to_string());
    }
    if resolved.recovered_from_proxy {
        let changed = persist_recovered_endpoint(&client, &server_name, &resolved).await?;
        if !changed {
            let latest_url = run_blocking_task({
                let client = client.clone();
                let server_name = server_name.clone();
                move || read_mcp_server_url_for_client(&client, &server_name)
            })
            .await?;
            if validated_endpoint(&latest_url).as_deref() != Some(resolved.endpoint.as_str()) {
                return Err("MCP 配置在自动恢复期间发生变化，请重新发起授权".to_string());
            }
        }
    }
    let mcp_endpoint = resolved.endpoint.clone();
    let credential_key = normalized_credential_key(&mcp_endpoint);
    // 同一远端端点无论从哪个客户端发起，都共享一把操作锁，避免并行授权
    // 互相覆盖 Token、绑定和代理配置。
    let credential_guard = lock_credential_operation(&credential_key).await?;

    // 代理运行参数在签发 Token 前准备好，避免本地准备失败后留下孤立授权。
    let (port, secret) = get_or_init_proxy_runtime()?;
    let proxy = proxy_url_for_binding(port, &secret, &credential_key, &mcp_endpoint, &server_name);
    let expected_url = if resolved.recovered_from_proxy {
        mcp_endpoint.clone()
    } else {
        resolved.source_url.clone()
    };

    let store = get_mcp_oauth_store()?;
    let existing_cred = store.creds_by_key.get(&credential_key).cloned();
    let existing_refresh_failure = store.refresh_failures.get(&credential_key).cloned();
    let existing_client_id = existing_cred
        .as_ref()
        .filter(|_| can_reuse_client_id(existing_refresh_failure.as_deref()))
        .map(|cred| cred.client_id.clone());
    let should_authorize = existing_cred.as_ref().map_or(true, |cred| {
        let expired = cred.expires_at > 0 && cred.expires_at <= now_secs();
        expired || existing_refresh_failure.is_some()
    });

    let mut applied_credential = None;
    if should_authorize {
        let outcome = run_authorize(&mcp_endpoint, existing_client_id, cancelled.clone()).await?;
        if cancelled.load(Ordering::SeqCst) {
            discard_authorize_outcome(outcome);
            return Err("授权已取消".to_string());
        }
        let credential = McpOAuthCred {
            client_id: outcome.client_id,
            access_token: outcome.access_token,
            refresh_token: outcome.refresh_token,
            expires_at: outcome.expires_at,
            last_refresh_attempt_at: now_secs(),
            auth_endpoint: outcome.auth_endpoint,
            token_endpoint: outcome.token_endpoint,
            revocation_endpoint: outcome.revocation_endpoint,
            mcp_endpoint: mcp_endpoint.clone(),
            resource: outcome.resource,
        };
        if let Err(error) = upsert_mcp_oauth_cred(&credential_key, credential.clone()) {
            discard_credential_tokens(credential);
            return Err(error);
        }
        applied_credential = Some(credential);
    }

    // 新 token 一旦完成持久化就进入原子提交阶段，继续完成代理与绑定，
    // 避免留下“凭据已保存但配置未绑定”的半完成状态。
    if !should_authorize && cancelled.load(Ordering::SeqCst) {
        return Err("授权已取消".to_string());
    }

    let updated_result = run_blocking_task({
        let client = client.clone();
        let server_name = server_name.clone();
        let expected_url = expected_url.clone();
        let proxy = proxy.clone();
        move || {
            write_mcp_server_url_for_client_if_current(&client, &server_name, &expected_url, &proxy)
        }
    })
    .await;
    let updated = match updated_result {
        Ok(updated) => updated,
        Err(error) => {
            if let Err(rollback_error) = rollback_authorized_credential(
                &credential_key,
                applied_credential.as_ref(),
                existing_cred.as_ref(),
                existing_refresh_failure.as_deref(),
            ) {
                return Err(format!(
                    "OAuth 代理配置提交失败: {error}；凭据回滚失败: {rollback_error}"
                ));
            }
            return Err(format!("OAuth 代理配置提交失败: {error}"));
        }
    };
    if !updated {
        if let Err(rollback_error) = rollback_authorized_credential(
            &credential_key,
            applied_credential.as_ref(),
            existing_cred.as_ref(),
            existing_refresh_failure.as_deref(),
        ) {
            return Err(format!(
                "MCP 配置在授权期间发生变化，未覆盖当前地址；凭据回滚失败: {rollback_error}"
            ));
        }
        return Err("MCP 配置在授权期间发生变化，未覆盖当前地址".to_string());
    }

    let displaced = match replace_mcp_oauth_binding(&client, &server_name, Some(&credential_key)) {
        Ok(displaced) => displaced,
        Err(error) => {
            let restored = run_blocking_task({
                let client = client.clone();
                let server_name = server_name.clone();
                let proxy = proxy.clone();
                let expected_url = expected_url.clone();
                move || {
                    write_mcp_server_url_for_client_if_current(
                        &client,
                        &server_name,
                        &proxy,
                        &expected_url,
                    )
                }
            })
            .await;
            let credential_rollback = rollback_authorized_credential(
                &credential_key,
                applied_credential.as_ref(),
                existing_cred.as_ref(),
                existing_refresh_failure.as_deref(),
            );
            let mut message = match restored {
                Ok(true) => format!("OAuth 绑定提交失败，配置已回滚: {error}"),
                Ok(false) => {
                    format!("OAuth 绑定提交失败: {error}；配置同时被其他程序修改，未强制回滚")
                }
                Err(rollback_error) => {
                    format!("OAuth 绑定提交失败: {error}；配置回滚也失败: {rollback_error}")
                }
            };
            if let Err(rollback_error) = credential_rollback {
                message.push_str("；凭据回滚失败: ");
                message.push_str(&rollback_error);
            }
            return Err(message);
        }
    };
    drop(credential_guard);
    if let Some((_, credential)) = displaced {
        if let Err(error) = revoke_remote_credential(&credential).await {
            log::warn!("OAuth 授权提交后清理旧凭据失败: {error}");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_oauth_cancel_authorize_for_client(
    client: String,
    server_name: String,
) -> Result<(), String> {
    let key = binding_key(&client, &server_name);
    let cancelled = pending_authorizations()
        .lock()
        .map_err(|_| "MCP OAuth 授权状态锁已损坏".to_string())?
        .get(&key)
        .cloned();

    if let Some(cancelled) = cancelled {
        cancelled.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_oauth_status_for_client(
    client: String,
    server_name: String,
) -> Result<McpOAuthStatus, String> {
    let store = get_mcp_oauth_store()?;
    let key = binding_key(&client, &server_name);
    let current_url = run_blocking_task({
        let client = client.clone();
        let server_name = server_name.clone();
        move || read_mcp_server_url_for_client(&client, &server_name)
    })
    .await?;

    if let Some(credential_key) = store.server_bindings.get(&key) {
        if let Some(mut status) = authorized_status(&store, credential_key) {
            let proxy_matches = recover_proxy_credential_key(&store, &current_url, &server_name)
                .as_deref()
                == Some(credential_key.as_str());
            if !proxy_matches {
                status.message = Some(match status.message {
                    Some(message) => format!(
                        "{message}；授权凭据已保存，但客户端地址未连接本地代理，请重新授权修复"
                    ),
                    None => {
                        "授权凭据已保存，但客户端地址未连接本地代理，请重新授权修复".to_string()
                    }
                });
            }
            return Ok(status);
        }
        return Ok(McpOAuthStatus {
            oauth_supported: Some(true),
            authorized: false,
            expires_at: 0,
            expiring_soon: false,
            expired: false,
            refresh_failed: false,
            needs_reauth: true,
            credential_key: Some(credential_key.clone()),
            message: Some("OAuth 绑定对应的本地凭据已丢失，请重新授权".to_string()),
        });
    }

    // 状态查询严格只读。旧代理修复、绑定恢复和 URL 改写由启动协调器或
    // 用户显式授权执行，避免仅打开页面就静默修改客户端配置。
    if let Some(credential_key) = recover_proxy_credential_key(&store, &current_url, &server_name) {
        if let Some(mut status) = authorized_status(&store, &credential_key) {
            status.authorized = false;
            status.message =
                Some("检测到可复用的本地授权但尚未绑定；点击授权即可完成修复".to_string());
            return Ok(status);
        }
    }

    let resolved = match resolve_real_endpoint(&client, &server_name, current_url).await {
        Ok(endpoint) => endpoint,
        Err(message) => {
            return Ok(McpOAuthStatus {
                oauth_supported: None,
                authorized: false,
                expires_at: 0,
                expiring_soon: false,
                expired: false,
                refresh_failed: false,
                needs_reauth: false,
                credential_key: None,
                message: Some(message),
            });
        }
    };

    let reusable_key = normalized_credential_key(&resolved.endpoint);
    if store.creds_by_key.contains_key(&reusable_key) {
        if let Some(mut status) = authorized_status(&store, &reusable_key) {
            status.authorized = false;
            status.message =
                Some("已有可复用授权；点击授权会完成客户端绑定，无需重复登录".to_string());
            return Ok(status);
        }
    }

    let recovery_message = resolved
        .recovered_from_proxy
        .then_some("检测到失效的本地代理配置；点击授权将先恢复真实地址".to_string());

    Ok(
        match tokio::time::timeout(
            Duration::from_secs(8),
            probe_oauth_endpoints(&resolved.endpoint),
        )
        .await
        {
            Ok(Ok(OAuthProbeResult::Supported(_))) => McpOAuthStatus {
                oauth_supported: Some(true),
                authorized: false,
                expires_at: 0,
                expiring_soon: false,
                expired: false,
                refresh_failed: false,
                needs_reauth: false,
                credential_key: None,
                message: recovery_message.clone(),
            },
            Ok(Ok(OAuthProbeResult::NotAdvertised)) => McpOAuthStatus {
                oauth_supported: Some(false),
                authorized: false,
                expires_at: 0,
                expiring_soon: false,
                expired: false,
                refresh_failed: false,
                needs_reauth: false,
                credential_key: None,
                message: Some(if resolved.recovered_from_proxy {
                    "检测到失效的本地代理配置；该服务未声明 OAuth，需显式修复真实地址".to_string()
                } else {
                    "该服务未声明 OAuth，可直接连接".to_string()
                }),
            },
            Ok(Err(message)) => McpOAuthStatus {
                oauth_supported: None,
                authorized: false,
                expires_at: 0,
                expiring_soon: false,
                expired: false,
                refresh_failed: false,
                needs_reauth: false,
                credential_key: None,
                message: Some(message),
            },
            Err(_) => McpOAuthStatus {
                oauth_supported: None,
                authorized: false,
                expires_at: 0,
                expiring_soon: false,
                expired: false,
                refresh_failed: false,
                needs_reauth: false,
                credential_key: None,
                message: Some("OAuth 能力检测超时，请确认服务可访问".to_string()),
            },
        },
    )
}

#[tauri::command]
pub async fn mcp_oauth_refresh_for_client(
    client: String,
    server_name: String,
) -> Result<McpOAuthStatus, String> {
    let credential_key =
        get_mcp_oauth_binding(&client, &server_name)?.ok_or("该 MCP 服务器尚未绑定 OAuth 凭据")?;
    refresh_credential(&credential_key).await?;
    mcp_oauth_status_for_client(client, server_name).await
}

#[tauri::command]
pub async fn mcp_oauth_revoke_for_client(
    client: String,
    server_name: String,
) -> Result<String, String> {
    let current_url_result = run_blocking_task({
        let client = client.clone();
        let server_name = server_name.clone();
        move || read_mcp_server_url_for_client(&client, &server_name)
    })
    .await;
    // 与后台/代理刷新共用同一把凭据锁。这样撤销会等待已开始的 token
    // 轮换完成，并撤销最新 token；刷新也不会在本地删除后又签发孤儿 token。
    let credential_guard = match get_mcp_oauth_binding(&client, &server_name)? {
        Some(credential_key) => Some(lock_credential_operation(&credential_key).await?),
        None => None,
    };
    let details = get_mcp_oauth_binding_details(&client, &server_name)?;
    // 服务端撤销是尽力操作：网络不可用时仍要完成本机清理，避免用户
    // 被一个不可达的授权服务器永久锁在旧凭据上。
    let remote_result = match details.as_ref() {
        Some((_, cred, true)) => Some(revoke_remote_credential(cred).await),
        _ => None,
    };
    let expected_credential_key = details.as_ref().map(|(key, _, _)| key.as_str());
    let unbound =
        unbind_mcp_oauth_server_if_current(&client, &server_name, expected_credential_key)?;
    drop(credential_guard);
    if let Some((_, cred, removed_last)) = unbound {
        if let Ok(current_url) = &current_url_result {
            if is_mcp_oauth_proxy_url(current_url) {
                let current_url = current_url.clone();
                let restored = run_blocking_task(move || {
                    write_mcp_server_url_for_client_if_current(
                        &client,
                        &server_name,
                        &current_url,
                        &cred.mcp_endpoint,
                    )
                })
                .await?;
                if !restored {
                    return Err("授权已清理，但 MCP 配置在恢复真实地址前发生变化".to_string());
                }
            }
        }
        if !removed_last {
            let mut message = "已解除当前客户端绑定，共享 OAuth 授权仍保留".to_string();
            if let Err(error) = current_url_result {
                message.push_str(&format!("；客户端配置未恢复：{error}"));
            }
            return Ok(message);
        }
        let mut message = match remote_result {
            Some(Ok(RemoteRevokeResult::Revoked)) => "本机与服务端 OAuth 授权均已撤销".to_string(),
            Some(Ok(RemoteRevokeResult::Unsupported)) => {
                "本机授权已删除；该服务未提供可用的服务端撤销端点".to_string()
            }
            Some(Err(error)) => format!("本机授权已删除，但服务端撤销未完成：{error}"),
            None => "本机授权已删除".to_string(),
        };
        if let Err(error) = current_url_result {
            message.push_str(&format!("；客户端配置未恢复：{error}"));
        }
        return Ok(message);
    }

    let current_url = current_url_result?;
    if is_mcp_oauth_proxy_url(&current_url) {
        let resolved = resolve_real_endpoint(&client, &server_name, current_url).await?;
        let restored = run_blocking_task({
            let client = client.clone();
            let server_name = server_name.clone();
            let source_url = resolved.source_url;
            let endpoint = resolved.endpoint;
            move || {
                write_mcp_server_url_for_client_if_current(
                    &client,
                    &server_name,
                    &source_url,
                    &endpoint,
                )
            }
        })
        .await?;
        if !restored {
            return Err("MCP 配置在恢复真实地址期间发生变化，请重新操作".to_string());
        }
    }
    Ok("当前客户端没有本地 OAuth 绑定，已完成配置清理".to_string())
}

/// 启动时修复所有 SysPulse OAuth 代理配置：
/// - 凭据仍存在时刷新端口/secret，并升级为自描述 v2 URL；
/// - 凭据丢失时从 v2、其他客户端或 RFC 9728 元数据恢复真实端点。
pub async fn reconcile_mcp_oauth_proxy_configs() -> Result<usize, String> {
    let (entries, mut errors) = run_blocking_task(|| {
        let mut entries = Vec::new();
        let mut errors = Vec::new();
        for kind in [
            McpClientKind::Kiro,
            McpClientKind::Codex,
            McpClientKind::ClaudeCli,
        ] {
            match load_mcp_items_for_client(kind) {
                Ok(items) => {
                    entries.extend(items.into_iter().filter_map(|item| {
                        (matches!(item.server_type.as_str(), "url" | "http" | "sse")
                            && is_mcp_oauth_proxy_url(&item.detail))
                        .then_some((kind.as_key().to_string(), item))
                    }));
                }
                Err(error) => {
                    errors.push(format!("{} 配置读取失败: {error}", kind.as_key()));
                }
            }
        }
        Ok((entries, errors))
    })
    .await?;

    let mut repaired = 0;
    for (client, item) in entries {
        let resolved = match resolve_real_endpoint(&client, &item.name, item.detail.clone()).await {
            Ok(resolved) => resolved,
            Err(error) => {
                errors.push(format!("{}/{} 自动恢复失败: {error}", client, item.name));
                continue;
            }
        };

        if let Some(credential_key) = resolved.credential_key.clone() {
            let mut changed = false;
            let (port, secret) = match get_or_init_proxy_runtime() {
                Ok(runtime) => runtime,
                Err(error) => {
                    errors.push(format!(
                        "{}/{} 代理运行参数读取失败: {error}",
                        client, item.name
                    ));
                    continue;
                }
            };
            let proxy = proxy_url_for_binding(
                port,
                &secret,
                &credential_key,
                &resolved.endpoint,
                &item.name,
            );
            if proxy != resolved.source_url {
                let updated = run_blocking_task({
                    let client = client.clone();
                    let server_name = item.name.clone();
                    let source_url = resolved.source_url.clone();
                    move || {
                        write_mcp_server_url_for_client_if_current(
                            &client,
                            &server_name,
                            &source_url,
                            &proxy,
                        )
                    }
                })
                .await;
                match updated {
                    Ok(true) => changed = true,
                    Ok(false) => continue,
                    Err(error) => {
                        errors.push(format!(
                            "{}/{} 代理地址刷新失败: {error}",
                            client, item.name
                        ));
                        continue;
                    }
                }
            }
            match get_mcp_oauth_binding(&client, &item.name) {
                Ok(binding) if binding.as_deref() == Some(credential_key.as_str()) => {}
                Ok(_) => match bind_mcp_oauth_server(&client, &item.name, &credential_key) {
                    Ok(()) => changed = true,
                    Err(error) => {
                        errors.push(format!("{}/{} 绑定恢复失败: {error}", client, item.name));
                    }
                },
                Err(error) => {
                    errors.push(format!("{}/{} 绑定读取失败: {error}", client, item.name));
                }
            }
            repaired += usize::from(changed);
        } else {
            match persist_recovered_endpoint(&client, &item.name, &resolved).await {
                Ok(true) => repaired += 1,
                Ok(false) => {}
                Err(error) => {
                    errors.push(format!(
                        "{}/{} 真实地址写回失败: {error}",
                        client, item.name
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(repaired)
    } else {
        Err(format!(
            "已修复 {repaired} 项，另有 {} 项失败：{}",
            errors.len(),
            errors.join("；")
        ))
    }
}

#[tauri::command]
pub async fn mcp_oauth_authorize(server_key: String) -> Result<(), String> {
    mcp_oauth_authorize_for_client("kiro".to_string(), server_key).await
}

#[tauri::command]
pub async fn mcp_oauth_cancel_authorize(server_key: String) -> Result<(), String> {
    mcp_oauth_cancel_authorize_for_client("kiro".to_string(), server_key).await
}

#[tauri::command]
pub async fn mcp_oauth_status(server_key: String) -> Result<McpOAuthStatus, String> {
    mcp_oauth_status_for_client("kiro".to_string(), server_key).await
}

#[tauri::command]
pub async fn mcp_oauth_revoke(server_key: String) -> Result<String, String> {
    mcp_oauth_revoke_for_client("kiro".to_string(), server_key).await
}

#[cfg(test)]
mod tests {
    use super::{can_reuse_client_id, recover_proxy_endpoint};
    use std::sync::Arc;

    #[test]
    fn reuses_registration_unless_the_client_itself_is_invalid() {
        assert!(can_reuse_client_id(None));
        assert!(can_reuse_client_id(Some(
            r#"{\"error\":\"invalid_grant\"}"#
        )));
        assert!(!can_reuse_client_id(Some(
            r#"{\"error\":\"invalid_client\"}"#
        )));
        assert!(!can_reuse_client_id(Some("unauthorized_client")));
    }

    #[tokio::test]
    async fn recovers_current_proxy_without_credentials_via_rfc9728() {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let origin = format!("http://127.0.0.1:{port}");
        let expected_endpoint = format!("{origin}/mcp");
        let proxy = format!(
            "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/{}/notion",
            urlencoding::encode(&origin)
        );
        let responder = server.clone();
        let response_endpoint = expected_endpoint.clone();
        let handle = std::thread::spawn(move || {
            let request = responder.recv().unwrap();
            assert_eq!(request.url(), "/.well-known/oauth-protected-resource/mcp");
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap();
            request
                .respond(
                    tiny_http::Response::from_string(format!(
                        r#"{{"resource":"{response_endpoint}"}}"#
                    ))
                    .with_header(header),
                )
                .unwrap();
        });

        let peers = vec!["https://wrong.example.com/mcp".to_string()];
        let recovered = recover_proxy_endpoint(&proxy, "notion", &peers, None)
            .await
            .unwrap();
        assert_eq!(recovered, expected_endpoint);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn recovery_prefers_standard_mcp_metadata_path() {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let origin = format!("http://127.0.0.1:{port}");
        let expected_endpoint = format!("{origin}/mcp");
        let proxy = format!(
            "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/{}/cloudflare-api",
            urlencoding::encode(&origin)
        );
        let responder = server.clone();
        let response_endpoint = expected_endpoint.clone();
        let handle = std::thread::spawn(move || {
            let request = responder.recv().unwrap();
            assert_eq!(request.url(), "/.well-known/oauth-protected-resource/mcp");
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap();
            request
                .respond(
                    tiny_http::Response::from_string(format!(
                        r#"{{"resource":"{response_endpoint}"}}"#
                    ))
                    .with_header(header),
                )
                .unwrap();
        });

        let recovered = recover_proxy_endpoint(&proxy, "cloudflare-api", &[], None)
            .await
            .unwrap();
        assert_eq!(recovered, expected_endpoint);
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn legacy_proxy_uses_unique_same_named_peer_endpoint() {
        let proxy = "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/notion";
        let peers = vec!["https://mcp.notion.com/mcp".to_string()];
        assert_eq!(
            recover_proxy_endpoint(proxy, "notion", &peers, None)
                .await
                .unwrap(),
            "https://mcp.notion.com/mcp"
        );
    }

    #[tokio::test]
    async fn legacy_proxy_refuses_ambiguous_peer_endpoints() {
        let proxy = "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/notion";
        let peers = vec![
            "https://one.example.com/mcp".to_string(),
            "https://two.example.com/mcp".to_string(),
        ];
        let error = recover_proxy_endpoint(proxy, "notion", &peers, None)
            .await
            .unwrap_err();
        assert!(error.contains("多个不同"));
    }
}

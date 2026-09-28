use super::*;

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn expires_at_from(expires_in: Option<i64>) -> i64 {
    match expires_in {
        Some(s) if s > 0 => now_secs() + s,
        _ => 0,
    }
}

pub(super) fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) {
        Err("授权已取消".to_string())
    } else {
        Ok(())
    }
}

/// 授权码换 token
pub(super) async fn exchange_code_with_cancel(
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    resource: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<TokenResp, String> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
        ("resource", resource),
    ];
    post_token_with_cancel(token_endpoint, &params, cancelled).await
}

/// refresh_token 刷新；调用方负责处理 refresh_token 轮换（缺省则沿用旧值）
pub async fn refresh_access_token(
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
    resource: &str,
) -> Result<(String, Option<String>, i64), String> {
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("resource", resource),
    ];
    let t = post_token(token_endpoint, &params).await?;
    Ok((
        t.access_token,
        t.refresh_token,
        expires_at_from(t.expires_in),
    ))
}

pub(super) fn credential_refresh_locks() -> &'static StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>
{
    static LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

pub(super) fn credential_refresh_lock(credential_key: &str) -> Result<Arc<AsyncMutex<()>>, String> {
    let mut locks = credential_refresh_locks()
        .lock()
        .map_err(|_| "MCP OAuth 刷新锁已损坏".to_string())?;
    Ok(locks
        .entry(credential_key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone())
}

pub async fn lock_credential_operation(
    credential_key: &str,
) -> Result<OwnedMutexGuard<()>, String> {
    Ok(credential_refresh_lock(credential_key)?.lock_owned().await)
}

/// 对持久化凭据执行串行刷新。
/// `observed_credential` 用于代理/后台任务识别“排队期间已有其它请求刷新成功”，
/// 此时直接复用最新凭据，避免用已轮换的 refresh_token 再次换取而触发 invalid_grant。
pub async fn refresh_stored_credential(
    credential_key: &str,
    observed_credential: Option<&McpOAuthCred>,
) -> Result<McpOAuthCred, String> {
    let lock = credential_refresh_lock(credential_key)?;
    let _guard = lock.lock().await;

    let store = get_mcp_oauth_store()?;
    let cred = store
        .creds_by_key
        .get(credential_key)
        .cloned()
        .ok_or_else(|| "未找到共享 OAuth 凭据".to_string())?;

    if observed_credential.is_some_and(|observed| {
        observed.access_token != cred.access_token || observed.refresh_token != cred.refresh_token
    }) {
        return Ok(cred);
    }
    if let Some(message) = store.refresh_failures.get(credential_key) {
        if mcp_oauth_failure_needs_reauth(message) {
            return Err(message.clone());
        }
    }

    let refresh_token = cred
        .refresh_token
        .clone()
        .ok_or_else(|| "该凭据没有 refresh_token，需重新授权".to_string())?;
    let attempted_access_token = cred.access_token.clone();
    match refresh_access_token(
        &cred.token_endpoint,
        &cred.client_id,
        &refresh_token,
        &cred.resource,
    )
    .await
    {
        Ok((access_token, new_refresh, expires_at)) => {
            let updated = McpOAuthCred {
                access_token,
                refresh_token: new_refresh.or(cred.refresh_token),
                expires_at,
                last_refresh_attempt_at: now_secs(),
                ..cred
            };
            match upsert_mcp_oauth_cred_if_current(
                credential_key,
                &attempted_access_token,
                Some(&refresh_token),
                updated,
            )? {
                McpOAuthConditionalUpdate::Applied(latest)
                | McpOAuthConditionalUpdate::Changed(latest) => Ok(latest),
                McpOAuthConditionalUpdate::Missing => {
                    Err("MCP OAuth 凭据已在刷新期间被移除".to_string())
                }
            }
        }
        Err(error) => {
            let message = error.to_string();
            match set_mcp_oauth_refresh_failure_if_current(
                credential_key,
                &attempted_access_token,
                Some(&refresh_token),
                message.clone(),
            ) {
                Ok(McpOAuthConditionalUpdate::Changed(latest)) => Ok(latest),
                Ok(McpOAuthConditionalUpdate::Missing) => {
                    Err("MCP OAuth 凭据已在刷新期间被移除".to_string())
                }
                Ok(McpOAuthConditionalUpdate::Applied(_)) => {
                    if mcp_oauth_failure_needs_reauth(&message) {
                        log::warn!(
                            "MCP OAuth 刷新授权已失效 ({credential_key})，需要重新授权: {message}"
                        );
                    }
                    Err(message)
                }
                Err(store_error) => {
                    log::error!(
                        "记录 MCP OAuth token 刷新失败状态失败 ({credential_key}): {store_error}"
                    );
                    Err(format!("{message}；记录刷新失败状态失败: {store_error}"))
                }
            }
        }
    }
}

pub(super) async fn post_token(
    token_endpoint: &str,
    params: &[(&str, &str)],
) -> Result<TokenResp, String> {
    post_token_with_cancel(token_endpoint, params, None).await
}

pub(super) async fn post_token_with_cancel(
    token_endpoint: &str,
    params: &[(&str, &str)],
    cancelled: Option<&AtomicBool>,
) -> Result<TokenResp, String> {
    let resp = send_oauth_request(
        oauth_http_client()?.post(token_endpoint).form(params),
        cancelled,
    )
    .await
    .map_err(|error| {
        if is_cancelled_error(&error, cancelled) {
            "授权已取消".to_string()
        } else {
            format!("token 请求失败: {error}")
        }
    })?;
    let status = resp.status();
    let text = read_oauth_response_text(resp, cancelled, "读取 token 响应失败").await?;
    if !status.is_success() {
        return Err(format!("token 换取失败 ({status}): {text}"));
    }
    let mut token = serde_json::from_str::<TokenResp>(&text)
        .map_err(|e| format!("解析 token 响应失败: {e}"))?;
    if token.access_token.trim().is_empty() {
        return Err("token 响应缺少有效 access_token".to_string());
    }
    token.refresh_token = token
        .refresh_token
        .filter(|refresh_token| !refresh_token.trim().is_empty());
    Ok(token)
}

/// RFC 7009 撤销一个 access/refresh token。服务端对已经失效的 token
/// 通常返回 invalid_token，这种情况按幂等成功处理。
pub async fn revoke_token(
    revocation_endpoint: &str,
    client_id: &str,
    token: &str,
    token_type_hint: Option<&str>,
) -> Result<(), String> {
    let mut params = vec![("token", token), ("client_id", client_id)];
    if let Some(hint) = token_type_hint {
        params.push(("token_type_hint", hint));
    }
    let resp = oauth_http_client()?
        .post(revocation_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|error| format!("撤销请求失败: {error}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|error| format!("读取撤销响应失败: {error}"))?;
    if status.is_success() {
        return Ok(());
    }
    let lower = body.to_ascii_lowercase();
    if status == reqwest::StatusCode::BAD_REQUEST && lower.contains("invalid_token") {
        return Ok(());
    }
    Err(format!("服务端撤销失败 ({status}): {body}"))
}

/// 用户在 token 已签发的边界取消授权时，异步回收刚获得的 token，
/// 避免本机未保存凭据但服务端仍残留一个可用 grant。
pub(super) fn discard_tokens(
    revocation_endpoint: Option<String>,
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
) {
    let Some(endpoint) = revocation_endpoint else {
        return;
    };
    tokio::spawn(async move {
        if let Some(refresh_token) = refresh_token.as_deref() {
            if let Err(error) = tokio::time::timeout(
                Duration::from_secs(8),
                revoke_token(&endpoint, &client_id, refresh_token, Some("refresh_token")),
            )
            .await
            .map_err(|_| "请求超时".to_string())
            .and_then(|result| result)
            {
                log::warn!("取消授权后回收 refresh token 失败: {error}");
            }
        }
        if refresh_token
            .as_deref()
            .map_or(true, |refresh_token| refresh_token != access_token)
        {
            if let Err(error) = tokio::time::timeout(
                Duration::from_secs(8),
                revoke_token(&endpoint, &client_id, &access_token, Some("access_token")),
            )
            .await
            .map_err(|_| "请求超时".to_string())
            .and_then(|result| result)
            {
                log::warn!("取消授权后回收 access token 失败: {error}");
            }
        }
    });
}

pub fn discard_authorize_outcome(outcome: AuthorizeOutcome) {
    discard_tokens(
        outcome.revocation_endpoint,
        outcome.client_id,
        outcome.access_token,
        outcome.refresh_token,
    );
}

pub fn discard_credential_tokens(credential: McpOAuthCred) {
    discard_tokens(
        credential.revocation_endpoint,
        credential.client_id,
        credential.access_token,
        credential.refresh_token,
    );
}

// ===== 本地回调服务器（PKCE 授权码交互）=====

// 远程 MCP 服务器 OAuth (RFC 8414 发现 + RFC 7591 DCR + PKCE 授权码)
// 用于在本应用内为 url 型 MCP 服务器（如 Notion）完成授权，
// Token 存入 SysPulse AI 管理凭据存储，由本地反代注入 Bearer。

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::ai::backend::oauth_store::{
    canonical_oauth_resource, get_mcp_oauth_store, mcp_oauth_failure_needs_reauth,
    set_mcp_oauth_refresh_failure_if_current, upsert_mcp_oauth_cred_if_current,
    McpOAuthConditionalUpdate, McpOAuthCred,
};
use crate::ai::backend::utils::browser::open_browser_keep_session;

/// 授权流程产出：换到 token + 端点信息，供命令层落盘
pub struct AuthorizeOutcome {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    pub resource: String,
}

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub resource: String,
}

#[derive(Debug, Clone)]
pub enum OAuthProbeResult {
    Supported(Endpoints),
    NotAdvertised,
}

#[derive(Deserialize)]
struct MetadataResp {
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    registration_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct ProtectedResourceMetadataResp {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
}

#[derive(Clone)]
struct CachedOAuthProbe {
    checked_at: Instant,
    result: Result<OAuthProbeResult, String>,
}

#[derive(Deserialize)]
struct RegisterResp {
    client_id: String,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

fn oauth_http_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    match CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("初始化 MCP OAuth HTTP 客户端失败: {e}"))
    }) {
        Ok(client) => Ok(client),
        Err(error) => Err(error.clone()),
    }
}

/// 发送 OAuth HTTP 请求，并允许授权流程在请求尚未完成时被取消。
///
/// reqwest future 被取消后会释放底层请求；这里用短间隔监听原子标记，
/// 因此取消不必等到默认的 30 秒 HTTP 超时才生效。普通状态/刷新请求
/// 传入 `None`，保持原有行为。
async fn send_oauth_request(
    request: reqwest::RequestBuilder,
    cancelled: Option<&AtomicBool>,
) -> Result<reqwest::Response, String> {
    let send = request.send();
    tokio::pin!(send);

    let Some(cancelled) = cancelled else {
        return send
            .await
            .map_err(|error| format!("OAuth 请求失败: {error}"));
    };

    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("授权已取消".to_string());
        }
        tokio::select! {
            result = &mut send => {
                return result.map_err(|error| format!("OAuth 请求失败: {error}"));
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

async fn read_oauth_response_text(
    response: reqwest::Response,
    cancelled: Option<&AtomicBool>,
    context: &str,
) -> Result<String, String> {
    let receive = response.text();
    tokio::pin!(receive);
    let Some(cancelled) = cancelled else {
        return receive.await.map_err(|error| format!("{context}: {error}"));
    };

    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("授权已取消".to_string());
        }
        tokio::select! {
            result = &mut receive => {
                return result.map_err(|error| format!("{context}: {error}"));
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

fn is_cancelled_error(error: &str, cancelled: Option<&AtomicBool>) -> bool {
    cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) || error == "授权已取消"
}

fn base64_url(data: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.encode(data)
}

/// 生成 PKCE (code_verifier, code_challenge)
fn gen_pkce() -> (String, String) {
    use rand::Rng;
    use sha2::{Digest, Sha256};
    let bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
    let verifier = base64_url(&bytes);
    let challenge = base64_url(&Sha256::new().chain_update(verifier.as_bytes()).finalize());
    (verifier, challenge)
}

/// 取 URL 的 scheme://host[:port]
fn origin_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            let scheme = u.scheme().to_string();
            u.host_str().map(|h| match u.port() {
                Some(p) => format!("{scheme}://{h}:{p}"),
                None => format!("{scheme}://{h}"),
            })
        })
        .unwrap_or_else(|| url.trim_end_matches('/').to_string())
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|current| current == &value) {
        values.push(value);
    }
}

fn validate_oauth_endpoint(value: &str, field: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value).map_err(|error| format!("OAuth {field} 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("OAuth {field} 必须是 http/https 地址"));
    }
    Ok(value.to_string())
}

fn protected_resource_metadata_urls(base_url: &str) -> Result<Vec<String>, String> {
    let parsed = url::Url::parse(base_url).map_err(|error| format!("MCP URL 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("MCP URL 必须是 http/https 地址".to_string());
    }

    let origin = origin_of(base_url);
    let path = parsed.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        push_unique(
            &mut urls,
            format!("{origin}/.well-known/oauth-protected-resource{path}"),
        );
    }
    push_unique(
        &mut urls,
        format!("{origin}/.well-known/oauth-protected-resource"),
    );
    Ok(urls)
}

fn authorization_server_metadata_urls(issuer: &str) -> Result<Vec<String>, String> {
    let parsed = url::Url::parse(issuer)
        .map_err(|error| format!("OAuth authorization server URL 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("OAuth authorization server 必须是 http/https 地址".to_string());
    }

    let origin = origin_of(issuer);
    let path = parsed.path().trim_end_matches('/');
    let issuer = issuer.trim_end_matches('/');
    let mut urls = Vec::new();
    push_unique(
        &mut urls,
        format!("{origin}/.well-known/oauth-authorization-server{path}"),
    );
    push_unique(
        &mut urls,
        format!("{issuer}/.well-known/openid-configuration"),
    );
    Ok(urls)
}

async fn discover_authorization_server(
    client: &reqwest::Client,
    issuer: &str,
    resource: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<Option<Endpoints>, String> {
    let urls = authorization_server_metadata_urls(issuer)?;
    let mut failures = Vec::new();
    let mut not_found = 0;
    for url in &urls {
        if let Some(cancelled) = cancelled {
            ensure_not_cancelled(cancelled)?;
        }
        match send_oauth_request(client.get(url), cancelled).await {
            Ok(resp) if resp.status().is_success() => {
                let text = read_oauth_response_text(
                    resp,
                    cancelled,
                    &format!("读取授权服务器元数据失败 ({url})"),
                )
                .await?;
                match serde_json::from_str::<MetadataResp>(&text) {
                    Ok(meta) => {
                        if let (Some(auth), Some(token)) = (
                            meta.authorization_endpoint.as_deref(),
                            meta.token_endpoint.as_deref(),
                        ) {
                            let auth = validate_oauth_endpoint(auth, "authorization_endpoint")?;
                            let token = validate_oauth_endpoint(token, "token_endpoint")?;
                            let registration = meta
                                .registration_endpoint
                                .as_deref()
                                .map(|value| {
                                    validate_oauth_endpoint(value, "registration_endpoint")
                                })
                                .transpose()?;
                            let revocation = meta
                                .revocation_endpoint
                                .as_deref()
                                .map(|value| validate_oauth_endpoint(value, "revocation_endpoint"))
                                .transpose()?;
                            return Ok(Some(Endpoints {
                                authorization_endpoint: auth,
                                token_endpoint: token,
                                registration_endpoint: registration,
                                revocation_endpoint: revocation,
                                resource: resource.to_string(),
                            }));
                        }
                        failures.push(format!("{url} 缺少 authorization_endpoint/token_endpoint"));
                    }
                    Err(error) => failures.push(format!("{url} 元数据无效: {error}")),
                }
            }
            Ok(resp)
                if matches!(
                    resp.status(),
                    reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
                ) =>
            {
                not_found += 1;
            }
            Ok(resp) => failures.push(format!("{url} 返回 {}", resp.status())),
            Err(error) if is_cancelled_error(&error, cancelled) => {
                return Err("授权已取消".to_string());
            }
            Err(error) => failures.push(format!("{url} 请求失败: {error}")),
        }
    }

    if not_found == urls.len() {
        Ok(None)
    } else {
        Err(format!(
            "OAuth 授权服务器元数据发现失败：{}",
            failures.join("；")
        ))
    }
}

async fn lookup_protected_resource_metadata(
    client: &reqwest::Client,
    base_url: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<(Option<ProtectedResourceMetadataResp>, Vec<String>), String> {
    let mut failures = Vec::new();
    for metadata_url in protected_resource_metadata_urls(base_url)? {
        if let Some(cancelled) = cancelled {
            ensure_not_cancelled(cancelled)?;
        }
        match send_oauth_request(client.get(&metadata_url), cancelled).await {
            Ok(resp) if resp.status().is_success() => {
                let text = read_oauth_response_text(
                    resp,
                    cancelled,
                    &format!("读取受保护资源元数据失败 ({metadata_url})"),
                )
                .await?;
                let metadata = serde_json::from_str::<ProtectedResourceMetadataResp>(&text)
                    .map_err(|error| format!("受保护资源元数据无效 ({metadata_url}): {error}"))?;
                let resource_url = url::Url::parse(&metadata.resource)
                    .map_err(|error| format!("OAuth resource URL 无效: {error}"))?;
                if !matches!(resource_url.scheme(), "http" | "https")
                    || resource_url.host_str().is_none()
                {
                    return Err("OAuth resource 必须是 http/https 地址".to_string());
                }
                return Ok((Some(metadata), failures));
            }
            Ok(resp)
                if matches!(
                    resp.status(),
                    reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
                ) => {}
            Ok(resp) => failures.push(format!(
                "受保护资源元数据 {metadata_url} 返回 {}",
                resp.status()
            )),
            Err(error) if is_cancelled_error(&error, cancelled) => {
                return Err("授权已取消".to_string());
            }
            Err(error) => {
                failures.push(format!("受保护资源元数据 {metadata_url} 请求失败: {error}"))
            }
        }
    }
    Ok((None, failures))
}

/// 只读取 RFC 9728 的 `resource`，用于在本地凭据丢失后恢复旧代理 URL
/// 对应的完整 MCP 端点，不依赖授权服务器元数据仍然可用。
pub async fn discover_protected_resource(base_url: &str) -> Result<Option<String>, String> {
    discover_protected_resource_with_cancel(base_url, None).await
}

pub(crate) async fn discover_protected_resource_with_cancel(
    base_url: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<Option<String>, String> {
    let (metadata, failures) =
        lookup_protected_resource_metadata(oauth_http_client()?, base_url, cancelled).await?;
    if let Some(metadata) = metadata {
        return Ok(Some(metadata.resource));
    }
    if failures.is_empty() {
        Ok(None)
    } else {
        Err(format!("受保护资源元数据发现失败：{}", failures.join("；")))
    }
}

async fn probe_oauth_endpoints_uncached(
    base_url: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<OAuthProbeResult, String> {
    let client = oauth_http_client()?;
    let (metadata, resource_failures) =
        lookup_protected_resource_metadata(client, base_url, cancelled).await?;

    if let Some(metadata) = metadata {
        let resource = canonical_oauth_resource(base_url, &metadata.resource);
        let authorization_servers = if metadata.authorization_servers.is_empty() {
            vec![origin_of(&metadata.resource)]
        } else {
            metadata.authorization_servers
        };
        let mut authorization_failures = Vec::new();
        for issuer in authorization_servers {
            match discover_authorization_server(client, &issuer, &resource, cancelled).await {
                Ok(Some(endpoints)) => return Ok(OAuthProbeResult::Supported(endpoints)),
                Ok(None) => {
                    authorization_failures.push(format!("{issuer} 未提供 RFC 8414/OIDC 元数据"))
                }
                Err(error) if is_cancelled_error(&error, cancelled) => {
                    return Err("授权已取消".to_string());
                }
                Err(error) => authorization_failures.push(error),
            }
        }
        return Err(format!(
            "MCP 服务已声明 OAuth，但授权服务器元数据发现失败：{}",
            authorization_failures.join("；")
        ));
    }

    let resource = canonical_oauth_resource(base_url, base_url.trim_end_matches('/'));
    match discover_authorization_server(client, &origin_of(base_url), &resource, cancelled).await {
        Ok(Some(endpoints)) => Ok(OAuthProbeResult::Supported(endpoints)),
        Ok(None) if resource_failures.is_empty() => Ok(OAuthProbeResult::NotAdvertised),
        Ok(None) => Err(format!(
            "OAuth 能力检测失败：{}",
            resource_failures.join("；")
        )),
        Err(error) if is_cancelled_error(&error, cancelled) => Err("授权已取消".to_string()),
        Err(error) if resource_failures.is_empty() => Err(error),
        Err(error) => Err(format!(
            "OAuth 能力检测失败：{}；{error}",
            resource_failures.join("；")
        )),
    }
}

fn oauth_probe_cache() -> &'static StdMutex<HashMap<String, CachedOAuthProbe>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, CachedOAuthProbe>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

pub async fn probe_oauth_endpoints(base_url: &str) -> Result<OAuthProbeResult, String> {
    probe_oauth_endpoints_with_cancel(base_url, None).await
}

async fn probe_oauth_endpoints_with_cancel(
    base_url: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<OAuthProbeResult, String> {
    // 授权流程中的探测不能复用或污染全局缓存，否则取消产生的错误会
    // 短暂覆盖正常状态，也无法在请求中途响应取消。
    if let Some(cancelled) = cancelled {
        ensure_not_cancelled(cancelled)?;
    }
    // URL path/query 可能大小写敏感，不能把整个地址转小写后共用发现结果。
    let key = base_url.trim_end_matches('/').to_string();
    if cancelled.is_none() {
        if let Ok(cache) = oauth_probe_cache().lock() {
            if let Some(cached) = cache.get(&key) {
                let ttl = if cached.result.is_ok() {
                    Duration::from_secs(300)
                } else {
                    Duration::from_secs(15)
                };
                if cached.checked_at.elapsed() < ttl {
                    return cached.result.clone();
                }
            }
        }
    }

    let result = probe_oauth_endpoints_uncached(base_url, cancelled).await;
    if cancelled.is_none() {
        if let Ok(mut cache) = oauth_probe_cache().lock() {
            cache.insert(
                key,
                CachedOAuthProbe {
                    checked_at: Instant::now(),
                    result: result.clone(),
                },
            );
        }
    }
    result
}

/// MCP OAuth 发现：先按 RFC 9728 查受保护资源元数据，再按 RFC 8414/OIDC
/// 查授权服务器元数据。没有声明 OAuth 的普通远程 MCP 不会再被当作授权失败。
pub async fn discover_endpoints(base_url: &str) -> Result<Endpoints, String> {
    discover_endpoints_with_cancel(base_url, None).await
}

async fn discover_endpoints_with_cancel(
    base_url: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<Endpoints, String> {
    match probe_oauth_endpoints_with_cancel(base_url, cancelled).await? {
        OAuthProbeResult::Supported(endpoints) => Ok(endpoints),
        OAuthProbeResult::NotAdvertised => {
            Err("该 MCP 服务未声明 OAuth，可直接连接，无需授权".to_string())
        }
    }
}

/// RFC 7591 动态客户端注册，返回 client_id
pub async fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<String, String> {
    register_client_with_cancel(registration_endpoint, redirect_uri, None).await
}

async fn register_client_with_cancel(
    registration_endpoint: &str,
    redirect_uri: &str,
    cancelled: Option<&AtomicBool>,
) -> Result<String, String> {
    let body = serde_json::json!({
        "client_name": "SysPulse MCP",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let resp = send_oauth_request(
        oauth_http_client()?.post(registration_endpoint).json(&body),
        cancelled,
    )
    .await
    .map_err(|error| {
        if is_cancelled_error(&error, cancelled) {
            "授权已取消".to_string()
        } else {
            format!("DCR 请求失败: {error}")
        }
    })?;
    let status = resp.status();
    let text = read_oauth_response_text(resp, cancelled, "读取 DCR 响应失败").await?;
    if !status.is_success() {
        return Err(format!("DCR 失败 ({status}): {text}"));
    }
    serde_json::from_str::<RegisterResp>(&text)
        .map(|r| r.client_id)
        .map_err(|e| format!("解析 DCR 响应失败: {e}"))
}

fn build_authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
    resource: &str,
) -> String {
    let sep = if authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    format!(
        "{authorization_endpoint}{sep}response_type=code&client_id={}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&resource={}",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(state),
        urlencoding::encode(code_challenge),
        urlencoding::encode(resource),
    )
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn expires_at_from(expires_in: Option<i64>) -> i64 {
    match expires_in {
        Some(s) if s > 0 => now_secs() + s,
        _ => 0,
    }
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::SeqCst) {
        Err("授权已取消".to_string())
    } else {
        Ok(())
    }
}

/// 授权码换 token
async fn exchange_code_with_cancel(
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

fn credential_refresh_locks() -> &'static StdMutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn credential_refresh_lock(credential_key: &str) -> Result<Arc<AsyncMutex<()>>, String> {
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

async fn post_token(token_endpoint: &str, params: &[(&str, &str)]) -> Result<TokenResp, String> {
    post_token_with_cancel(token_endpoint, params, None).await
}

async fn post_token_with_cancel(
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
fn discard_tokens(
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

fn parse_callback(url: &str, expected_state: &str) -> Result<String, String> {
    let query = url.split('?').nth(1).unwrap_or("");
    let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    if params.get("state").map(String::as_str) != Some(expected_state) {
        return Err("state 不匹配".to_string());
    }
    if let Some(err) = params.get("error") {
        let desc = params.get("error_description").map_or("未知错误", |s| s);
        return Err(format!("{err}: {desc}"));
    }
    params
        .get("code")
        .cloned()
        .ok_or_else(|| "未收到授权码".to_string())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 启动本地服务器等待回调，返回授权码（阻塞，带 10 分钟超时）
fn wait_for_code(
    server: Arc<tiny_http::Server>,
    expected_state: &str,
    cancelled: &Arc<AtomicBool>,
) -> Result<String, String> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(600);
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("授权已取消".to_string());
        }
        if start.elapsed() > timeout {
            return Err("授权超时".to_string());
        }
        if let Ok(Some(request)) = server.try_recv() {
            let url = request.url().to_string();
            if url.starts_with("/oauth/callback") {
                let result = parse_callback(&url, expected_state);
                let page = match &result {
                    Ok(_) => "<html><body><h1>授权信息已收到</h1><p>应用正在完成授权，可以关闭此窗口</p></body></html>"
                        .to_string(),
                    Err(m) => format!(
                        "<html><body><h1>授权失败</h1><p>{}</p></body></html>",
                        escape_html(m)
                    ),
                };
                let mut response = tiny_http::Response::from_string(page);
                if let Ok(header) = tiny_http::Header::from_bytes(
                    &b"Content-Type"[..],
                    &b"text/html; charset=utf-8"[..],
                ) {
                    response = response.with_header(header);
                }
                let _ = request.respond(response);
                return result;
            }
            let _ = request.respond(tiny_http::Response::empty(404));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// 完整授权流程：发现 -> DCR(可复用 client_id) -> PKCE 授权 -> 换 token
pub async fn run_authorize(
    base_url: &str,
    existing_client_id: Option<String>,
    cancelled: Arc<AtomicBool>,
) -> Result<AuthorizeOutcome, String> {
    ensure_not_cancelled(&cancelled)?;
    let endpoints = discover_endpoints_with_cancel(base_url, Some(cancelled.as_ref())).await?;
    ensure_not_cancelled(&cancelled)?;
    let resource = endpoints.resource.clone();

    let server =
        tiny_http::Server::http("127.0.0.1:0").map_err(|e| format!("无法启动本地服务器: {e}"))?;
    let server = Arc::new(server);
    let port = server.server_addr().to_ip().map_or(0, |a| a.port());
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");

    let client_id = match existing_client_id {
        Some(id) if !id.is_empty() => id,
        _ => {
            let reg_ep = endpoints
                .registration_endpoint
                .clone()
                .ok_or("服务器未提供注册端点，无法完成 DCR")?;
            let client_id =
                register_client_with_cancel(&reg_ep, &redirect_uri, Some(cancelled.as_ref()))
                    .await?;
            ensure_not_cancelled(&cancelled)?;
            client_id
        }
    };

    let state = uuid::Uuid::new_v4().to_string();
    let (verifier, challenge) = gen_pkce();
    let authorize_url = build_authorize_url(
        &endpoints.authorization_endpoint,
        &client_id,
        &redirect_uri,
        &state,
        &challenge,
        &resource,
    );

    ensure_not_cancelled(&cancelled)?;
    open_browser_keep_session(&authorize_url)?;

    let code = {
        let state = state.clone();
        let cancelled = cancelled.clone();
        tokio::task::spawn_blocking(move || wait_for_code(server, &state, &cancelled))
            .await
            .map_err(|e| format!("授权任务异常: {e}"))??
    };

    ensure_not_cancelled(&cancelled)?;
    let token = exchange_code_with_cancel(
        &endpoints.token_endpoint,
        &client_id,
        &code,
        &verifier,
        &redirect_uri,
        &resource,
        Some(cancelled.as_ref()),
    )
    .await?;
    let outcome = AuthorizeOutcome {
        client_id,
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: expires_at_from(token.expires_in),
        auth_endpoint: endpoints.authorization_endpoint,
        token_endpoint: endpoints.token_endpoint,
        revocation_endpoint: endpoints.revocation_endpoint,
        resource,
    };
    // 取消可能发生在 token 请求返回的边界；在写入本地凭据前再次检查，
    // 避免用户明确取消后仍把新 token 持久化，并尽力回收服务端 grant。
    if cancelled.load(Ordering::SeqCst) {
        discard_authorize_outcome(outcome);
        Err("授权已取消".to_string())
    } else {
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_strips_path() {
        assert_eq!(
            origin_of("https://mcp.notion.com/mcp"),
            "https://mcp.notion.com"
        );
        assert_eq!(origin_of("https://h.io:8443/a/b"), "https://h.io:8443");
    }

    #[test]
    fn builds_mcp_and_authorization_metadata_urls() {
        assert_eq!(
            protected_resource_metadata_urls("https://mcp.figma.com/mcp").unwrap(),
            vec![
                "https://mcp.figma.com/.well-known/oauth-protected-resource/mcp",
                "https://mcp.figma.com/.well-known/oauth-protected-resource",
            ]
        );
        assert_eq!(
            authorization_server_metadata_urls("https://auth.example.com/tenant").unwrap(),
            vec![
                "https://auth.example.com/.well-known/oauth-authorization-server/tenant",
                "https://auth.example.com/tenant/.well-known/openid-configuration",
            ]
        );
    }

    #[tokio::test]
    async fn discovers_oauth_through_protected_resource_metadata() {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let origin = format!("http://127.0.0.1:{port}");
        let base_url = format!("{origin}/mcp");
        let responder = server.clone();
        let response_origin = origin.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..2 {
                let request = responder.recv().unwrap();
                let body = match request.url() {
                    "/.well-known/oauth-protected-resource/mcp" => format!(
                        r#"{{"resource":"{response_origin}/mcp","authorization_servers":["{response_origin}"]}}"#
                    ),
                    "/.well-known/oauth-authorization-server" => format!(
                        r#"{{"authorization_endpoint":"{response_origin}/authorize","token_endpoint":"{response_origin}/token","registration_endpoint":"{response_origin}/register"}}"#
                    ),
                    path => panic!("unexpected metadata path: {path}"),
                };
                let header =
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap();
                request
                    .respond(tiny_http::Response::from_string(body).with_header(header))
                    .unwrap();
            }
        });

        let result = probe_oauth_endpoints(&base_url).await.unwrap();
        let OAuthProbeResult::Supported(endpoints) = result else {
            panic!("expected OAuth support");
        };
        assert_eq!(endpoints.resource, base_url);
        assert_eq!(
            endpoints.authorization_endpoint,
            format!("{origin}/authorize")
        );
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn classifies_remote_mcp_without_metadata_as_no_oauth() {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}/mcp");
        let responder = server.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..4 {
                let request = responder.recv().unwrap();
                request.respond(tiny_http::Response::empty(404)).unwrap();
            }
        });

        assert!(matches!(
            probe_oauth_endpoints(&base_url).await.unwrap(),
            OAuthProbeResult::NotAdvertised
        ));
        handle.join().unwrap();
    }

    #[test]
    fn pkce_challenge_is_deterministic_per_verifier() {
        let (v, c) = gen_pkce();
        use sha2::{Digest, Sha256};
        let expect = base64_url(&Sha256::new().chain_update(v.as_bytes()).finalize());
        assert_eq!(c, expect);
    }

    #[test]
    fn authorize_url_picks_correct_separator() {
        let u = build_authorize_url(
            "https://a/auth",
            "cid",
            "http://127.0.0.1/cb",
            "st",
            "ch",
            "https://r",
        );
        assert!(u.starts_with("https://a/auth?response_type=code"));
        let u2 = build_authorize_url(
            "https://a/auth?x=1",
            "cid",
            "http://127.0.0.1/cb",
            "st",
            "ch",
            "https://r",
        );
        assert!(u2.contains("auth?x=1&response_type=code"));
    }

    #[test]
    fn notion_metadata_uses_origin_resource_for_authorization() {
        assert_eq!(
            canonical_oauth_resource("https://mcp.notion.com/mcp", "https://mcp.notion.com/mcp"),
            "https://mcp.notion.com"
        );
    }

    #[test]
    fn parse_callback_validates_state_and_code() {
        assert_eq!(
            parse_callback("/oauth/callback?code=abc&state=s1", "s1").unwrap(),
            "abc"
        );
        assert!(parse_callback("/oauth/callback?code=abc&state=s2", "s1").is_err());
        assert!(parse_callback("/oauth/callback?error=denied&state=s1", "s1").is_err());
    }

    #[test]
    fn expires_at_handles_missing() {
        assert_eq!(expires_at_from(None), 0);
        assert_eq!(expires_at_from(Some(0)), 0);
        assert!(expires_at_from(Some(3600)) > now_secs());
    }

    #[test]
    fn refreshes_for_the_same_credential_share_one_lock() {
        let first = credential_refresh_lock("https://mcp.cloudflare.com").unwrap();
        let second = credential_refresh_lock("https://mcp.cloudflare.com").unwrap();
        let other = credential_refresh_lock("https://mcp.example.com").unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }
}

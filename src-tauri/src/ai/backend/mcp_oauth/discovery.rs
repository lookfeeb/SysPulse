use super::*;

pub(super) fn origin_of(url: &str) -> String {
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

pub(super) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|current| current == &value) {
        values.push(value);
    }
}

pub(super) fn validate_oauth_endpoint(value: &str, field: &str) -> Result<String, String> {
    let parsed = url::Url::parse(value).map_err(|error| format!("OAuth {field} 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(format!("OAuth {field} 必须是 http/https 地址"));
    }
    Ok(value.to_string())
}

pub(super) fn protected_resource_metadata_urls(base_url: &str) -> Result<Vec<String>, String> {
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

pub(super) fn authorization_server_metadata_urls(issuer: &str) -> Result<Vec<String>, String> {
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

pub(super) async fn discover_authorization_server(
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

pub(super) async fn lookup_protected_resource_metadata(
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

pub(super) async fn probe_oauth_endpoints_uncached(
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

pub(super) fn oauth_probe_cache() -> &'static StdMutex<HashMap<String, CachedOAuthProbe>> {
    static CACHE: OnceLock<StdMutex<HashMap<String, CachedOAuthProbe>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

pub async fn probe_oauth_endpoints(base_url: &str) -> Result<OAuthProbeResult, String> {
    probe_oauth_endpoints_with_cancel(base_url, None).await
}

pub(super) async fn probe_oauth_endpoints_with_cancel(
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

pub(super) async fn discover_endpoints_with_cancel(
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

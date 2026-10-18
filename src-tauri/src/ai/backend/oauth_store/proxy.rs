use super::*;

/// 已知服务的稳定 MCP 端点兼容规则。
///
/// Notion 的路径级 RFC 9728 元数据会对任意输入路径回显一个 `resource`，
/// 即使该路径并不是可用的 MCP 端点（例如历史恢复出的 `/notion`）。
/// 官方 Notion MCP 配置使用 `/mcp`，因此在进入凭据、代理和配置写回前
/// 统一纠正，避免把服务名误当成真实路径长期持久化。
pub fn canonical_mcp_endpoint(endpoint: &str) -> String {
    let Ok(mut url) = url::Url::parse(endpoint) else {
        return endpoint.to_string();
    };
    if url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("mcp.notion.com"))
    {
        url.set_path("/mcp");
        url.set_fragment(None);
        return url.to_string();
    }
    endpoint.to_string()
}

/// OAuth 的 resource 与实际 MCP 请求端点是两个概念。Notion 官方配置
/// 明确使用 origin 作为 `oauth_resource`，而实际 MCP 请求仍发往 `/mcp`。
/// 这里也用于迁移此前错误保存为 `/notion` 或 `/mcp` 的 resource。
pub fn canonical_oauth_resource(mcp_endpoint: &str, advertised_resource: &str) -> String {
    if url::Url::parse(mcp_endpoint).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case("mcp.notion.com"))
    }) {
        return legacy_origin_credential_key(mcp_endpoint);
    }
    advertised_resource.to_string()
}

pub fn normalized_credential_key(endpoint: &str) -> String {
    let endpoint = canonical_mcp_endpoint(endpoint);
    if let Ok(mut url) = url::Url::parse(&endpoint) {
        if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
            url.set_fragment(None);
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&path);
            return url.to_string();
        }
    }

    endpoint.trim_end_matches('/').to_ascii_lowercase()
}

/// v0/v1 代理和早期凭据只按 origin 作为 key；仅用于兼容迁移，
/// 新凭据必须按完整 MCP 端点隔离，避免同域不同资源串用 token。
pub fn legacy_origin_credential_key(endpoint: &str) -> String {
    if let Ok(url) = url::Url::parse(endpoint) {
        if let Some(host) = url.host_str() {
            let scheme = url.scheme().to_ascii_lowercase();
            let host = host.to_ascii_lowercase();
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host
            };
            return match url.port() {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            };
        }
    }
    endpoint.trim_end_matches('/').to_ascii_lowercase()
}

pub fn credential_key_matches_endpoint(credential_key: &str, endpoint: &str) -> bool {
    credential_key == normalized_credential_key(endpoint)
        || credential_key == legacy_origin_credential_key(endpoint)
}

pub fn mcp_oauth_failure_needs_reauth(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "invalid_grant",
        "invalid_client",
        "unauthorized_client",
        "access_denied",
        "grant not found",
        "refresh token expired",
        "invalid refresh token",
        "refresh token is invalid",
        "revoked",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub fn encode_credential_key(credential_key: &str) -> String {
    urlencoding::encode(credential_key).into_owned()
}

pub fn decode_credential_key(credential_key: &str) -> String {
    urlencoding::decode(credential_key)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| credential_key.to_string())
}

pub(super) fn encode_proxy_endpoint(endpoint: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.encode(endpoint.as_bytes())
}

pub fn decode_proxy_endpoint(endpoint: &str) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let decoded = URL_SAFE_NO_PAD.decode(endpoint).ok()?;
    let endpoint = String::from_utf8(decoded).ok()?;
    let parsed = url::Url::parse(&endpoint).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.fragment().is_some()
    {
        return None;
    }
    Some(endpoint)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOAuthProxyInfo {
    pub credential_key: Option<String>,
    pub mcp_endpoint: Option<String>,
    pub server_name: String,
}

pub(super) fn is_loopback_proxy_url(url: &url::Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }

    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

pub(super) fn is_proxy_secret(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 解析 SysPulse 生成的本地 OAuth 反代地址。
///
/// - v0: `/{secret}/{server_name}`
/// - v1: `/{secret}/{credential_key}/{server_name}`
/// - v2: `/{secret}/v2/{credential_key}/{mcp_endpoint}/{server_name}`
///
/// v2 同时保存完整 MCP 端点，凭据库丢失后仍可无损恢复客户端配置。
pub fn parse_mcp_oauth_proxy_url(value: &str) -> Option<McpOAuthProxyInfo> {
    let Ok(url) = url::Url::parse(value) else {
        return None;
    };
    if !is_loopback_proxy_url(&url) {
        return None;
    }
    if url.query().is_some() || url.fragment().is_some() {
        return None;
    }

    let mut segments = url.path_segments()?.collect::<Vec<_>>();
    while segments.last().is_some_and(|segment| segment.is_empty()) {
        segments.pop();
    }
    let secret = *segments.first()?;
    if !is_proxy_secret(secret) {
        return None;
    }

    let decode_server_name = |value: &str| {
        urlencoding::decode(value)
            .ok()
            .map(|value| value.into_owned())
            .filter(|value| !value.is_empty())
    };

    match segments.as_slice() {
        [_, "v2", encoded_credential_key, encoded_endpoint, encoded_server_name] => {
            let encoded_key = decode_credential_key(encoded_credential_key);
            let parsed_key = url::Url::parse(&encoded_key).ok()?;
            if !matches!(parsed_key.scheme(), "http" | "https") || parsed_key.host_str().is_none() {
                return None;
            }
            let mcp_endpoint = decode_proxy_endpoint(encoded_endpoint)?;
            let credential_key = normalized_credential_key(&mcp_endpoint);
            let parsed_encoded_key = if parsed_key.path() == "/"
                && parsed_key.query().is_none()
                && parsed_key.fragment().is_none()
            {
                legacy_origin_credential_key(&encoded_key)
            } else {
                normalized_credential_key(&encoded_key)
            };
            if parsed_encoded_key != credential_key
                && parsed_encoded_key != legacy_origin_credential_key(&mcp_endpoint)
            {
                return None;
            }
            Some(McpOAuthProxyInfo {
                credential_key: Some(credential_key),
                mcp_endpoint: Some(mcp_endpoint),
                server_name: decode_server_name(encoded_server_name)?,
            })
        }
        [_, encoded_credential_key, encoded_server_name] => {
            let credential_key = decode_credential_key(encoded_credential_key);
            let parsed_key = url::Url::parse(&credential_key).ok()?;
            if !matches!(parsed_key.scheme(), "http" | "https") || parsed_key.host_str().is_none() {
                return None;
            }
            let credential_key = if parsed_key.path() == "/" && parsed_key.query().is_none() {
                legacy_origin_credential_key(&credential_key)
            } else {
                normalized_credential_key(&credential_key)
            };
            Some(McpOAuthProxyInfo {
                credential_key: Some(credential_key),
                mcp_endpoint: None,
                server_name: decode_server_name(encoded_server_name)?,
            })
        }
        [_, encoded_server_name] => Some(McpOAuthProxyInfo {
            credential_key: None,
            mcp_endpoint: None,
            server_name: decode_server_name(encoded_server_name)?,
        }),
        _ => None,
    }
}

/// 判断 URL 是否为 SysPulse 生成的本地 OAuth 反代地址。
pub fn is_mcp_oauth_proxy_url(value: &str) -> bool {
    parse_mcp_oauth_proxy_url(value).is_some()
}

/// 从当前格式的本地反代 URL 中恢复凭据键。
pub fn proxy_credential_key_from_url(value: &str, server_name: &str) -> Option<String> {
    let proxy = parse_mcp_oauth_proxy_url(value)?;
    if proxy.server_name != server_name {
        return None;
    }
    proxy.credential_key
}

pub(super) fn endpoint_matches_server_name(endpoint: &str, server_name: &str) -> bool {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
                .and_then(|segment| urlencoding::decode(segment).ok())
                .map(|segment| segment == server_name)
        })
        .unwrap_or(false)
}

/// 从当前或历史本地反代 URL 恢复仍存在于凭据存储中的 credential key。
pub fn recover_proxy_credential_key(
    store: &McpOAuthStore,
    value: &str,
    server_name: &str,
) -> Option<String> {
    if let Some(credential_key) = proxy_credential_key_from_url(value, server_name) {
        if store.creds_by_key.contains_key(&credential_key) {
            return Some(credential_key);
        }
        let mut legacy_matches = store
            .creds_by_key
            .iter()
            .filter(|(_, credential)| {
                legacy_origin_credential_key(&credential.mcp_endpoint) == credential_key
            })
            .map(|(key, _)| key.clone());
        if let Some(matched) = legacy_matches.next() {
            if legacy_matches.next().is_none() {
                return Some(matched);
            }
        }
    }
    if !is_mcp_oauth_proxy_url(value) {
        return None;
    }

    let mut matches = store
        .creds_by_key
        .iter()
        .filter(|(_, credential)| {
            endpoint_matches_server_name(&credential.mcp_endpoint, server_name)
        })
        .map(|(credential_key, _)| credential_key.clone());
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

pub fn binding_key(client: &str, server_name: &str) -> String {
    format!("{}:{}", client.to_ascii_lowercase(), server_name)
}

pub fn proxy_url_for_binding(
    port: u16,
    secret: &str,
    credential_key: &str,
    mcp_endpoint: &str,
    server_name: &str,
) -> String {
    format!(
        "http://127.0.0.1:{port}/{secret}/v2/{}/{}/{}",
        encode_credential_key(credential_key),
        encode_proxy_endpoint(mcp_endpoint),
        urlencoding::encode(server_name)
    )
}

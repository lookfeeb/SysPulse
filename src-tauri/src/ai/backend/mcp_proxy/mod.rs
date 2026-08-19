// 本地 MCP 反向代理
// mcp.json 的 url 无法携带认证头，故让其指向本地版本化代理地址，
// 本服务注入自动刷新的 Authorization: Bearer 后转发到真实上游 MCP 地址，并流式回传。
//
// 安全：仅绑定 127.0.0.1；路径中的 <secret> 做本地校验，防止本机其他进程盗用 token。

use axum::{
    body::Body,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use std::collections::HashMap;
use std::time::Duration;

use crate::ai::backend::mcp_oauth::refresh_stored_credential;
use crate::ai::backend::oauth_store::{
    credential_key_matches_endpoint, decode_credential_key, decode_proxy_endpoint,
    get_mcp_oauth_store, get_or_init_proxy_runtime, legacy_origin_credential_key,
    mcp_oauth_failure_needs_reauth, normalized_credential_key, set_proxy_runtime_port,
    McpOAuthCred,
};

#[derive(Clone)]
struct ProxyState {
    secret: String,
    client: reqwest::Client,
}

const MAX_PROXY_REQUEST_BODY: usize = 8 * 1024 * 1024;

fn proxy_router(state: ProxyState) -> Router {
    Router::new()
        .route(
            "/{secret}/v2/{credential_key}/{mcp_endpoint}/{server_name}",
            any(handle),
        )
        .route(
            "/{secret}/v2/{credential_key}/{mcp_endpoint}/{server_name}/{*rest}",
            any(handle),
        )
        // 兼容 v1 地址；启动修复会把仍有效的配置升级为 v2。
        .route("/{secret}/{credential_key}/{server_name}", any(handle))
        .route(
            "/{secret}/{credential_key}/{server_name}/{*rest}",
            any(handle),
        )
        .with_state(state)
}

/// 启动本地反代，返回监听端口（已持久化，跨重启稳定）
pub async fn start_proxy() -> Result<u16, String> {
    let (port, secret) = get_or_init_proxy_runtime()?;
    let state = ProxyState {
        secret,
        // SSE 响应不能设置全局请求超时，但连接阶段必须有上限，避免坏端点
        // 长时间占住本地代理任务。
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| format!("初始化 MCP 代理 HTTP 客户端失败: {error}"))?,
    };

    let app = proxy_router(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(first_error) => {
            let fallback_addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
            let listener = tokio::net::TcpListener::bind(fallback_addr)
                .await
                .map_err(|fallback_error| {
                    format!(
                        "反代绑定 {addr} 失败: {first_error}；自动分配新端口也失败: {fallback_error}"
                    )
                })?;
            let replacement_port = listener
                .local_addr()
                .map_err(|error| format!("读取新反代端口失败: {error}"))?
                .port();
            set_proxy_runtime_port(replacement_port)?;
            log::warn!(
                "MCP 反代端口 {port} 不可用，已自动切换到 {replacement_port}: {first_error}"
            );
            listener
        }
    };
    let bound_port = listener
        .local_addr()
        .map_err(|error| format!("读取反代监听地址失败: {error}"))?
        .port();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("MCP 反代退出: {e}");
        }
    });
    log::info!("MCP 反代已启动: http://127.0.0.1:{bound_port}");
    Ok(bound_port)
}

async fn handle(
    State(st): State<ProxyState>,
    Path(params): Path<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let secret = params.get("secret").cloned().unwrap_or_default();
    let requested_credential_key = params
        .get("credential_key")
        .map(|value| decode_credential_key(value))
        .unwrap_or_default();
    let server_name = params.get("server_name").cloned().unwrap_or_default();
    let rest = params.get("rest").cloned();

    if secret != st.secret {
        return text_resp(StatusCode::FORBIDDEN, "invalid proxy secret");
    }

    let described_endpoint = if let Some(encoded_endpoint) = params.get("mcp_endpoint") {
        let Some(endpoint) = decode_proxy_endpoint(encoded_endpoint) else {
            return text_resp(StatusCode::BAD_REQUEST, "invalid proxy endpoint");
        };
        if !credential_key_matches_endpoint(&requested_credential_key, &endpoint) {
            return text_resp(StatusCode::BAD_REQUEST, "proxy endpoint mismatch");
        }
        Some(endpoint)
    } else {
        None
    };

    let Ok(store) = get_mcp_oauth_store() else {
        return text_resp(StatusCode::INTERNAL_SERVER_ERROR, "read store failed");
    };
    let credential_key = if store.creds_by_key.contains_key(&requested_credential_key) {
        requested_credential_key
    } else if let Some(endpoint) = described_endpoint.as_deref() {
        normalized_credential_key(endpoint)
    } else {
        let mut candidates = store
            .creds_by_key
            .iter()
            .filter(|(_, credential)| {
                legacy_origin_credential_key(&credential.mcp_endpoint) == requested_credential_key
            })
            .map(|(key, _)| key.clone());
        match candidates.next() {
            None => requested_credential_key,
            Some(candidate) if candidates.next().is_none() => candidate,
            Some(_) => {
                return text_resp(
                    StatusCode::CONFLICT,
                    "ambiguous legacy OAuth credential key; re-authorize this MCP server",
                )
            }
        }
    };
    let Some(cred) = store.creds_by_key.get(&credential_key).cloned() else {
        return text_resp(StatusCode::NOT_FOUND, "unknown credential key");
    };
    if store
        .refresh_failures
        .get(&credential_key)
        .is_some_and(|message| mcp_oauth_failure_needs_reauth(message))
    {
        return text_resp(StatusCode::UNAUTHORIZED, "MCP OAuth 已失效，请重新授权");
    }

    // 拼接上游 URL：保留端点自身 query，并追加客户端请求的子路径/query。
    let target = match build_upstream_url(&cred.mcp_endpoint, rest.as_deref(), uri.query()) {
        Ok(target) => target,
        Err(error) => return text_resp(StatusCode::BAD_REQUEST, &error),
    };

    // 读取一次 body 字节（便于 401 重试时复用）
    if headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_PROXY_REQUEST_BODY)
    {
        return text_resp(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    }
    let body_bytes = match axum::body::to_bytes(body, MAX_PROXY_REQUEST_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return text_resp(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body invalid or too large",
            )
        }
    };

    let mut access_token = cred.access_token.clone();
    let mut resp = forward(
        &st.client,
        &method,
        &target,
        &headers,
        &body_bytes,
        &access_token,
    )
    .await;

    // 上游 401：尝试刷新一次后重试
    if let Ok(r) = &resp {
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(new_token) = try_refresh(&credential_key, &server_name, &cred).await {
                access_token = new_token;
                resp = forward(
                    &st.client,
                    &method,
                    &target,
                    &headers,
                    &body_bytes,
                    &access_token,
                )
                .await;
            }
        }
    }

    match resp {
        Ok(upstream) => stream_back(upstream),
        Err(e) => text_resp(StatusCode::BAD_GATEWAY, &format!("upstream error: {e}")),
    }
}

fn build_upstream_url(
    base_url: &str,
    rest: Option<&str>,
    incoming_query: Option<&str>,
) -> Result<String, String> {
    let mut target = url::Url::parse(base_url)
        .map_err(|error| format!("invalid upstream MCP endpoint: {error}"))?;
    if let Some(rest) = rest.filter(|value| !value.is_empty()) {
        let mut segments = target
            .path_segments_mut()
            .map_err(|_| "upstream MCP endpoint cannot accept a subpath".to_string())?;
        segments.pop_if_empty();
        segments.extend(rest.split('/').filter(|segment| !segment.is_empty()));
    }
    if let Some(query) = incoming_query.filter(|value| !value.is_empty()) {
        target
            .query_pairs_mut()
            .extend_pairs(url::form_urlencoded::parse(query.as_bytes()));
    }
    Ok(target.to_string())
}

/// 转发请求到上游，注入 Bearer，透传方法/头/体
async fn forward(
    client: &reqwest::Client,
    method: &Method,
    target: &str,
    headers: &HeaderMap,
    body: &[u8],
    access_token: &str,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut req = client.request(method.clone(), target);
    for (name, value) in headers {
        let n = name.as_str().to_ascii_lowercase();
        // 这些头由 reqwest/我们重置，不透传
        if matches!(n.as_str(), "host" | "authorization" | "content-length") {
            continue;
        }
        req = req.header(name, value);
    }
    req = req.header("Authorization", format!("Bearer {access_token}"));
    if !body.is_empty() {
        req = req.body(body.to_vec());
    }
    req.send().await
}

/// 刷新 token 并持久化（处理 refresh_token 轮换），返回新 access_token
async fn try_refresh(
    credential_key: &str,
    server_name: &str,
    cred: &McpOAuthCred,
) -> Option<String> {
    cred.refresh_token.as_ref()?;
    if get_mcp_oauth_store()
        .ok()
        .and_then(|store| store.refresh_failures.get(credential_key).cloned())
        .as_deref()
        .is_some_and(mcp_oauth_failure_needs_reauth)
    {
        // invalid_grant/Grant not found 已记录为需重新授权，后续请求不再重复换取
        // 已失效的 grant，也不再刷屏输出 ERROR。
        return None;
    }

    match refresh_stored_credential(credential_key, Some(cred)).await {
        Ok(updated) => Some(updated.access_token),
        Err(error) => {
            let message = error.to_string();
            if !mcp_oauth_failure_needs_reauth(&message) {
                log::error!("MCP token 刷新失败 ({server_name}): {message}");
            }
            None
        }
    }
}

/// 将上游响应（含 SSE 流）转回客户端
fn stream_back(upstream: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        let n = name.as_str().to_ascii_lowercase();
        if matches!(
            n.as_str(),
            "transfer-encoding" | "content-length" | "connection"
        ) {
            continue;
        }
        builder = builder.header(name, value);
    }
    let stream = upstream.bytes_stream();
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| text_resp(StatusCode::INTERNAL_SERVER_ERROR, "build response failed"))
}

fn text_resp(status: StatusCode, msg: &str) -> Response {
    let mut resp = Response::new(Body::from(msg.to_string()));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        "Content-Type",
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::{build_upstream_url, proxy_router, ProxyState};

    #[test]
    fn versioned_and_legacy_proxy_routes_can_coexist() {
        let _ = proxy_router(ProxyState {
            secret: "test".to_string(),
            client: reqwest::Client::new(),
        });
    }

    #[test]
    fn upstream_url_preserves_base_and_incoming_query() {
        let target = build_upstream_url(
            "https://example.com/mcp?tenant=workspace",
            Some("tools/list"),
            Some("cursor=a%2Fb"),
        )
        .unwrap();
        let parsed = url::Url::parse(&target).unwrap();
        assert_eq!(parsed.path(), "/mcp/tools/list");
        assert_eq!(
            parsed.query_pairs().collect::<Vec<_>>(),
            vec![
                ("tenant".into(), "workspace".into()),
                ("cursor".into(), "a/b".into())
            ]
        );
    }
}

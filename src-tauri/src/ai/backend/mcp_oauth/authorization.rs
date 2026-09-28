use super::*;

pub(super) fn base64_url(data: &[u8]) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD.encode(data)
}

/// 生成 PKCE (code_verifier, code_challenge)
pub(super) fn gen_pkce() -> (String, String) {
    use rand::Rng;
    use sha2::{Digest, Sha256};
    let bytes: Vec<u8> = (0..32).map(|_| rand::thread_rng().gen()).collect();
    let verifier = base64_url(&bytes);
    let challenge = base64_url(&Sha256::new().chain_update(verifier.as_bytes()).finalize());
    (verifier, challenge)
}

/// 取 URL 的 scheme://host[:port]
/// RFC 7591 动态客户端注册，返回 client_id
pub(super) async fn register_client_with_cancel(
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

pub(super) fn build_authorize_url(
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

pub(super) fn parse_callback(url: &str, expected_state: &str) -> Result<String, String> {
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

pub(super) fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// 启动本地服务器等待回调，返回授权码（阻塞，带 10 分钟超时）
pub(super) fn wait_for_code(
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

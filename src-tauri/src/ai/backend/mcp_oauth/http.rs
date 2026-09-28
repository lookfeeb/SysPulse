use super::*;

pub(super) fn oauth_http_client() -> Result<&'static reqwest::Client, String> {
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
pub(super) async fn send_oauth_request(
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

pub(super) async fn read_oauth_response_text(
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

pub(super) fn is_cancelled_error(error: &str, cancelled: Option<&AtomicBool>) -> bool {
    cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) || error == "授权已取消"
}

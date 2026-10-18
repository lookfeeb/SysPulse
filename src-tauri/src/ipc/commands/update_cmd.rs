use base64::Engine;
use futures_util::{stream, StreamExt, TryStreamExt};
use reqwest::{header, Client, Response, StatusCode};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, Manager, ResourceId, Webview};
use tauri_plugin_updater::Update;

const MAX_BYTES: u64 = 512 * 1024 * 1024;
const PARALLEL_THRESHOLD: u64 = 8 * 1024 * 1024;
const CONNECTIONS: usize = 4;
static INSTALL_LOCK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone, Serialize, specta::Type)]
#[serde(tag = "event", content = "data")]
pub enum UpdateDownloadEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        content_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    Verifying,
    Installing,
}

struct Progress {
    channel: Channel<UpdateDownloadEvent>,
    state: Mutex<(u64, Instant)>,
}

impl Progress {
    fn start(&self, total: Option<u64>) {
        *self.state.lock().unwrap() = (0, Instant::now());
        let _ = self.channel.send(UpdateDownloadEvent::Started {
            content_length: total,
        });
    }

    fn add(&self, bytes: usize, total: Option<u64>) {
        let mut state = self.state.lock().unwrap();
        state.0 += bytes as u64;
        if state.1.elapsed() >= Duration::from_millis(200) || total == Some(state.0) {
            let _ = self.channel.send(UpdateDownloadEvent::Progress {
                downloaded: state.0,
                content_length: total,
            });
            state.1 = Instant::now();
        }
    }
}

fn content_range(response: &Response, start: u64, end: u64) -> Option<u64> {
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (actual_start, actual_end) = range.split_once('-')?;
    let total = total.parse::<u64>().ok()?;
    (actual_start.parse::<u64>().ok()? == start
        && actual_end.parse::<u64>().ok()? == end
        && end < total
        && total <= MAX_BYTES)
        .then_some(total)
}

fn ranges(total: u64) -> Vec<(u64, u64)> {
    let chunk = total / CONNECTIONS as u64;
    (0..CONNECTIONS)
        .map(|index| {
            let start = index as u64 * chunk;
            let end = if index + 1 == CONNECTIONS {
                total - 1
            } else {
                start + chunk - 1
            };
            (start, end)
        })
        .collect()
}

async fn read_body(
    response: Response,
    expected: Option<u64>,
    mut on_chunk: impl FnMut(usize),
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if bytes.len() as u64 + chunk.len() as u64 > expected.unwrap_or(MAX_BYTES) {
            return Err("更新包长度超过预期".into());
        }
        on_chunk(chunk.len());
        bytes.extend_from_slice(&chunk);
    }
    if expected.is_some_and(|size| size != bytes.len() as u64) {
        return Err("更新包下载不完整".into());
    }
    Ok(bytes)
}

// A one-byte GET discovers range support and the final CDN URL in one request.
// Servers that ignore Range can stream the original response without downloading twice.
async fn download(
    client: &Client,
    url: &url::Url,
    on_start: impl Fn(Option<u64>),
    on_chunk: impl Fn(usize, Option<u64>) + Sync,
) -> Result<Vec<u8>, String> {
    let probe = client
        .get(url.clone())
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if probe.status() == StatusCode::OK {
        let total = probe.content_length();
        if total.is_some_and(|size| size > MAX_BYTES) {
            return Err("更新包过大".into());
        }
        on_start(total);
        return read_body(probe, total, |bytes| on_chunk(bytes, total)).await;
    }
    let total = content_range(&probe, 0, 0).ok_or("服务端未返回有效的分段下载响应")?;
    let final_url = probe.url().clone();
    let validator = probe
        .headers()
        .get(header::ETAG)
        .filter(|value| !value.as_bytes().starts_with(b"W/"))
        .or_else(|| probe.headers().get(header::LAST_MODIFIED))
        .cloned();
    read_body(probe, Some(1), |_| {}).await?;
    on_start(Some(total));
    if total < PARALLEL_THRESHOLD {
        let response = client
            .get(final_url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        return read_body(response, Some(total), |bytes| on_chunk(bytes, Some(total))).await;
    }
    let parts: Vec<Vec<u8>> = stream::iter(ranges(total))
        .map(|(start, end)| {
            let client = client.clone();
            let final_url = final_url.clone();
            let validator = validator.clone();
            let on_chunk = &on_chunk;
            async move {
                let mut request = client
                    .get(final_url)
                    .header(header::RANGE, format!("bytes={start}-{end}"));
                if let Some(validator) = validator {
                    request = request.header(header::IF_RANGE, validator);
                }
                let response = request.send().await.map_err(|e| e.to_string())?;
                if content_range(&response, start, end) != Some(total) {
                    return Err("分段范围或更新包长度发生变化".into());
                }
                read_body(response, Some(end - start + 1), |bytes| {
                    on_chunk(bytes, Some(total))
                })
                .await
            }
        })
        .buffered(CONNECTIONS)
        .try_collect()
        .await?;
    Ok(parts.concat())
}

fn verify(bytes: &[u8], signature: &str, public_key: &str) -> Result<(), String> {
    let decode = |value: &str| -> Result<String, String> {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(value)
            .map_err(|e| e.to_string())?;
        String::from_utf8(decoded).map_err(|e| e.to_string())
    };
    let key =
        minisign_verify::PublicKey::decode(&decode(public_key)?).map_err(|e| e.to_string())?;
    let signature =
        minisign_verify::Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(bytes, &signature, true)
        .map_err(|e| format!("更新签名校验失败：{e}"))
}

#[tauri::command]
#[specta::specta]
pub async fn download_install_update(
    webview: Webview,
    rid: ResourceId,
    on_event: Channel<UpdateDownloadEvent>,
) -> Result<(), String> {
    if webview.label() != "config" {
        return Err("更新仅可从设置窗口发起".into());
    }
    let _permit = INSTALL_LOCK.try_acquire().map_err(|_| "更新正在进行中")?;
    // Resolve the native resource, never accept an arbitrary installer URL or key from JS.
    let mut update = (*webview
        .resources_table()
        .get::<Update>(rid)
        .map_err(|e| e.to_string())?)
    .clone();
    let public_key = webview
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(|key| key.as_str())
        .ok_or("缺少更新校验公钥")?
        .to_owned();
    let mut headers = update.headers.clone();
    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("identity"),
    );
    let mut builder = Client::builder()
        .user_agent(concat!("SysPulse/", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600));
    if update.no_proxy {
        builder = builder.no_proxy();
    } else if let Some(proxy) = &update.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(|e| e.to_string())?);
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let progress = Arc::new(Progress {
        channel: on_event.clone(),
        state: Mutex::new((0, Instant::now())),
    });
    let bytes = match download(
        &client,
        &update.download_url,
        |total| progress.start(total),
        |bytes, total| progress.add(bytes, total),
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, "Parallel update download failed; retrying with the standard updater");
            progress.start(None);
            update.timeout = Some(Duration::from_secs(600));
            // Keep the plugin's verified single-stream path for proxies/CDNs that reject ranges.
            update
                .download(|bytes, total| progress.add(bytes, total), || {})
                .await
                .map_err(|e| e.to_string())?
        }
    };
    let _ = on_event.send(UpdateDownloadEvent::Verifying);
    // Update::install itself does not verify. This must precede every install path.
    verify(&bytes, &update.signature, &public_key)?;
    let _ = on_event.send(UpdateDownloadEvent::Installing);
    tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;

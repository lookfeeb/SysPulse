use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Local synthetic HTTP server; never downloads or installs a real release.
async fn fixture(
    mode: &'static str,
    size: usize,
) -> (url::Url, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url =
        url::Url::parse(&format!("http://{}/update", listener.local_addr().unwrap())).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let requests = count.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let len = socket.read(&mut buf).await.unwrap();
                    if len == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..len]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8(request).unwrap().to_lowercase();
                let range = text
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="));
                let (start, end, partial) = if mode != "ignore" {
                    range
                        .map(|range| {
                            let (a, b) = range.split_once('-').unwrap();
                            (
                                a.parse::<usize>().unwrap(),
                                b.parse::<usize>().unwrap(),
                                true,
                            )
                        })
                        .unwrap_or((0, size - 1, false))
                } else {
                    (0, size - 1, false)
                };
                let length = end - start + 1;
                let range_header = if partial {
                    let reported_start = if mode == "bad-range" && end > 0 {
                        start + 1
                    } else {
                        start
                    };
                    format!("Content-Range: bytes {reported_start}-{end}/{size}\r\n")
                } else {
                    String::new()
                };
                let status = if partial {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let headers = format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\n{range_header}Connection: close\r\n\r\n");
                if socket.write_all(headers.as_bytes()).await.is_err() {
                    return;
                }
                let sent = if mode == "truncate" {
                    length / 2
                } else {
                    length
                };
                let body: Vec<_> = (start..start + sent).map(|i| (i % 251) as u8).collect();
                let _ = socket.write_all(&body).await;
            });
        }
    });
    (url, count, server)
}

#[tokio::test]
async fn parallel_ranges_reassemble_in_order_and_report_exact_bytes() {
    let size = PARALLEL_THRESHOLD as usize + 17;
    let (url, requests, server) = fixture("ranges", size).await;
    let received = AtomicUsize::new(0);
    let result = download(
        &Client::builder().no_proxy().build().unwrap(),
        &url,
        |total| assert_eq!(total, Some(size as u64)),
        |bytes, _| {
            received.fetch_add(bytes, Ordering::SeqCst);
        },
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(requests.load(Ordering::SeqCst), CONNECTIONS + 1);
    assert_eq!(received.load(Ordering::SeqCst), size);
    assert_eq!(
        result,
        (0..size).map(|i| (i % 251) as u8).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn server_ignoring_ranges_uses_one_stream() {
    let (url, requests, server) = fixture("ignore", 1024).await;
    let result = download(
        &Client::builder().no_proxy().build().unwrap(),
        &url,
        |_| {},
        |_, _| {},
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(result.len(), 1024);
}

#[tokio::test]
async fn small_updates_use_a_single_body_request() {
    let (url, requests, server) = fixture("ranges", 1024).await;
    let result = download(
        &Client::builder().no_proxy().build().unwrap(),
        &url,
        |_| {},
        |_, _| {},
    )
    .await
    .unwrap();
    server.abort();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(result.len(), 1024);
}

#[tokio::test]
async fn changed_ranges_and_truncated_bodies_are_rejected() {
    for mode in ["bad-range", "truncate"] {
        let (url, _, server) = fixture(mode, PARALLEL_THRESHOLD as usize).await;
        let result = download(
            &Client::builder().no_proxy().build().unwrap(),
            &url,
            |_| {},
            |_, _| {},
        )
        .await;
        server.abort();
        assert!(
            result.is_err(),
            "{mode} must not produce an installable download"
        );
    }
}

#[test]
fn malformed_signatures_cannot_pass_verification() {
    assert!(verify(b"installer", "invalid", "invalid").is_err());
}
#[test]
fn signature_check_accepts_valid_data_and_rejects_tampering() {
    // Public test vector published by minisign-verify (no private key).
    let key = "untrusted comment: minisign public key E7620F1842B4E81F\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let signature = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";
    let key = base64::engine::general_purpose::STANDARD.encode(key);
    let signature = base64::engine::general_purpose::STANDARD.encode(signature);
    assert!(verify(b"test", &signature, &key).is_ok());
    assert!(verify(b"Test", &signature, &key).is_err());
}

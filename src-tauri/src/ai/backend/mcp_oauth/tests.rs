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

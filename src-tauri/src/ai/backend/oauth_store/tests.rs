use super::{
    binding_key, canonical_mcp_endpoint, canonical_oauth_resource, credential_matches,
    decode_mcp_oauth_store_value, is_mcp_oauth_proxy_url, mcp_oauth_failure_needs_reauth,
    normalize_mcp_oauth_store, normalized_credential_key, parse_mcp_oauth_proxy_url,
    proxy_credential_key_from_url, proxy_mcp_endpoint_from_url, proxy_url_for_binding,
    recover_proxy_credential_key, rollback_credential_in_store_if_current, McpOAuthCred,
    McpOAuthCredentialRollback, McpOAuthStore,
};

fn credential(access_token: &str, refresh_token: Option<&str>) -> McpOAuthCred {
    McpOAuthCred {
        client_id: "client".to_string(),
        access_token: access_token.to_string(),
        refresh_token: refresh_token.map(str::to_string),
        expires_at: 0,
        last_refresh_attempt_at: 0,
        auth_endpoint: "https://example.com/authorize".to_string(),
        token_endpoint: "https://example.com/token".to_string(),
        revocation_endpoint: None,
        mcp_endpoint: "https://example.com/mcp".to_string(),
        resource: "https://example.com".to_string(),
    }
}

#[test]
fn recognizes_permanent_refresh_grant_errors() {
    assert!(mcp_oauth_failure_needs_reauth(
        r#"token 换取失败 (400 Bad Request): {"error":"invalid_grant","error_description":"Grant not found"}"#
    ));
    assert!(mcp_oauth_failure_needs_reauth(
        "refresh token revoked by server"
    ));
    assert!(mcp_oauth_failure_needs_reauth(
        r#"{"error":"invalid_client"}"#
    ));
    assert!(!mcp_oauth_failure_needs_reauth(
        "token 请求失败: connection reset"
    ));
}

#[test]
fn conditional_refresh_checks_access_and_rotating_refresh_tokens() {
    let current = credential("access-1", Some("refresh-2"));
    assert!(credential_matches(&current, "access-1", Some("refresh-2")));
    assert!(!credential_matches(&current, "access-0", Some("refresh-2")));
    assert!(!credential_matches(&current, "access-1", Some("refresh-1")));
}

#[test]
fn authorization_rollback_restores_shared_credential_and_failure_snapshot() {
    let key = "https://example.com/mcp";
    let old = credential("old-access", Some("old-refresh"));
    let new = credential("new-access", Some("new-refresh"));
    let mut store = McpOAuthStore::default();
    store.creds_by_key.insert(key.to_string(), new.clone());
    store
        .server_bindings
        .insert("codex:server".to_string(), key.to_string());

    let result = rollback_credential_in_store_if_current(
        &mut store,
        key,
        &new.access_token,
        new.refresh_token.as_deref(),
        Some(old.clone()),
        Some("旧刷新失败".to_string()),
    );
    assert!(matches!(result, McpOAuthCredentialRollback::Restored(_)));
    let restored = store.creds_by_key.get(key).unwrap();
    assert!(credential_matches(
        restored,
        &old.access_token,
        old.refresh_token.as_deref()
    ));
    assert_eq!(
        store.refresh_failures.get(key).map(String::as_str),
        Some("旧刷新失败")
    );
}

#[test]
fn authorization_rollback_never_removes_a_new_credential_after_it_is_bound() {
    let key = "https://example.com/mcp";
    let new = credential("new-access", Some("new-refresh"));
    let mut store = McpOAuthStore::default();
    store.creds_by_key.insert(key.to_string(), new.clone());
    store
        .server_bindings
        .insert("kiro:server".to_string(), key.to_string());

    let result = rollback_credential_in_store_if_current(
        &mut store,
        key,
        &new.access_token,
        new.refresh_token.as_deref(),
        None,
        None,
    );
    assert!(matches!(result, McpOAuthCredentialRollback::InUse));
    assert!(store.creds_by_key.contains_key(key));
}

#[test]
fn detects_legacy_plaintext_store_for_automatic_migration() {
    let json = serde_json::to_string(&McpOAuthStore::default()).unwrap();
    let (store, needs_encryption) = decode_mcp_oauth_store_value(&json).unwrap();
    assert!(needs_encryption);
    assert!(store.creds_by_key.is_empty());
}

#[test]
fn proxy_url_round_trip_recovers_credential_key_and_full_endpoint() {
    let mcp_endpoint = "https://mcp.notion.com/mcp?tenant=workspace";
    let credential_key = normalized_credential_key(mcp_endpoint);
    let proxy = proxy_url_for_binding(
        18_796,
        "99dbba166d4343388b813bb294a8af4b",
        &credential_key,
        mcp_endpoint,
        "notion",
    );
    assert!(is_mcp_oauth_proxy_url(&proxy));
    assert_eq!(
        proxy_credential_key_from_url(&proxy, "notion").as_deref(),
        Some(credential_key.as_str())
    );
    assert_eq!(
        proxy_mcp_endpoint_from_url(&proxy, "notion").as_deref(),
        Some(mcp_endpoint)
    );
    assert!(proxy_credential_key_from_url(&proxy, "other").is_none());
    assert!(!is_mcp_oauth_proxy_url("http://127.0.0.1:18796/mcp"));
}

#[test]
fn current_proxy_url_remains_backward_compatible() {
    let proxy = "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/https%3A%2F%2Fmcp.notion.com/notion";
    let parsed = parse_mcp_oauth_proxy_url(proxy).unwrap();
    assert_eq!(
        parsed.credential_key.as_deref(),
        Some("https://mcp.notion.com")
    );
    assert_eq!(parsed.mcp_endpoint, None);
    assert_eq!(parsed.server_name, "notion");
}

#[test]
fn legacy_proxy_url_recovers_a_unique_matching_credential() {
    let mut store = McpOAuthStore::default();
    let credential_key = "https://mcp.notion.com/notion".to_string();
    store.creds_by_key.insert(
        credential_key.clone(),
        credential("access", Some("refresh")),
    );
    store
        .creds_by_key
        .get_mut(&credential_key)
        .unwrap()
        .mcp_endpoint = "https://mcp.notion.com/notion".to_string();

    let legacy = "http://127.0.0.1:18796/99dbba166d4343388b813bb294a8af4b/notion";
    assert_eq!(
        recover_proxy_credential_key(&store, legacy, "notion").as_deref(),
        Some(credential_key.as_str())
    );
}

#[test]
fn different_resources_on_one_origin_have_isolated_credential_keys() {
    assert_ne!(
        normalized_credential_key("https://example.com/mcp/tenant-a"),
        normalized_credential_key("https://example.com/mcp/tenant-b")
    );
}

#[test]
fn canonicalizes_notion_endpoint_and_oauth_resource() {
    assert_eq!(
        canonical_mcp_endpoint("https://mcp.notion.com/notion"),
        "https://mcp.notion.com/mcp"
    );
    assert_eq!(
        canonical_oauth_resource("https://mcp.notion.com/mcp", "https://mcp.notion.com/mcp"),
        "https://mcp.notion.com"
    );
}

#[test]
fn migrates_origin_key_and_bindings_to_full_endpoint_key() {
    let mut store = McpOAuthStore::default();
    let mut cred = credential("access", Some("refresh"));
    cred.mcp_endpoint = "https://example.com/mcp/tenant-a".to_string();
    store
        .creds_by_key
        .insert("https://example.com".to_string(), cred);
    store.server_bindings.insert(
        binding_key("codex", "tenant-a"),
        "https://example.com".to_string(),
    );

    assert!(normalize_mcp_oauth_store(&mut store));
    let full_key = "https://example.com/mcp/tenant-a";
    assert!(store.creds_by_key.contains_key(full_key));
    assert_eq!(
        store.server_bindings[&binding_key("codex", "tenant-a")],
        full_key
    );
    assert!(!normalize_mcp_oauth_store(&mut store));
}

#[cfg(windows)]
#[test]
fn dpapi_store_round_trip_hides_tokens_at_rest() {
    let mut store = McpOAuthStore::default();
    store.creds_by_key.insert(
        "https://example.com".to_string(),
        credential("access-secret", Some("refresh-secret")),
    );
    let encoded = super::encode_mcp_oauth_store_value(&store).unwrap();
    assert!(encoded.starts_with(super::DPAPI_PREFIX));
    assert!(!encoded.contains("access-secret"));
    assert!(!encoded.contains("refresh-secret"));

    let (decoded, needs_encryption) = decode_mcp_oauth_store_value(&encoded).unwrap();
    assert!(!needs_encryption);
    assert_eq!(
        decoded.creds_by_key["https://example.com"].access_token,
        "access-secret"
    );
}

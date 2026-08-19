use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::ai::backend::runtime::data_dir;

const DPAPI_PREFIX: &str = "dpapi:v1:";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthCred {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    /// 最近一次刷新尝试的 Unix 时间；用于 expires_in 缺失时的保守定时续期。
    #[serde(default)]
    pub last_refresh_attempt_at: i64,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    /// RFC 7009 revocation endpoint. Older credentials may not have this
    /// value; in that case revocation falls back to local cleanup only.
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    pub mcp_endpoint: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStore {
    #[serde(default)]
    pub creds: HashMap<String, McpOAuthCred>,
    #[serde(default)]
    pub creds_by_key: HashMap<String, McpOAuthCred>,
    #[serde(default)]
    pub server_bindings: HashMap<String, String>,
    #[serde(default)]
    pub refresh_failures: HashMap<String, String>,
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub proxy_secret: Option<String>,
}

fn mcp_oauth_path() -> PathBuf {
    data_dir().join("mcp-oauth.json")
}

fn mcp_oauth_store_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_mcp_oauth_store() -> Result<MutexGuard<'static, ()>, String> {
    mcp_oauth_store_lock()
        .lock()
        .map_err(|_| "MCP OAuth 凭据存储锁已损坏".to_string())
}

fn load_mcp_oauth_store_unlocked() -> Result<McpOAuthStore, String> {
    let stored = crate::ai::backend::db::kv_get("mcp_oauth", "store")?;
    let (mut store, needs_encryption, migrated_legacy) = if let Some(value) = stored {
        let (store, plaintext) = decode_mcp_oauth_store_value(&value)?;
        (store, plaintext, false)
    } else if let Some(store) = load_legacy_mcp_oauth()? {
        (store, false, true)
    } else {
        (McpOAuthStore::default(), false, false)
    };

    let normalized = normalize_mcp_oauth_store(&mut store);
    if needs_encryption || migrated_legacy || normalized {
        save_mcp_oauth_store_unlocked(&store)?;
    }
    if needs_encryption {
        if let Err(error) = crate::ai::backend::db::compact_after_sensitive_migration() {
            log::warn!("MCP OAuth 明文已转换为 DPAPI 密文，但清理 SQLite 旧页失败: {error}");
        }
    }
    if migrated_legacy {
        archive_legacy_mcp_oauth();
    }
    secure_legacy_mcp_oauth_files_once();

    Ok(store)
}

fn save_mcp_oauth_store_unlocked(store: &McpOAuthStore) -> Result<(), String> {
    let content = encode_mcp_oauth_store_value(store)?;
    crate::ai::backend::db::kv_set("mcp_oauth", "store", &content)
}

pub fn get_mcp_oauth_store() -> Result<McpOAuthStore, String> {
    let _guard = lock_mcp_oauth_store()?;
    load_mcp_oauth_store_unlocked()
}

fn load_legacy_mcp_oauth() -> Result<Option<McpOAuthStore>, String> {
    let path = mcp_oauth_path();
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取旧 MCP OAuth 文件失败 ({}): {e}", path.display()))?;
    let (store, _) = decode_mcp_oauth_store_value(&content)
        .map_err(|e| format!("解析旧 MCP OAuth 文件失败 ({}): {e}", path.display()))?;
    Ok(Some(store))
}

fn archive_legacy_mcp_oauth() {
    let path = mcp_oauth_path();
    let backup = path.with_extension("json.bak");
    if !path.exists() || backup.exists() {
        return;
    }
    if let Err(error) = std::fs::rename(&path, &backup) {
        log::warn!(
            "备份旧 MCP OAuth 文件失败 ({} -> {}): {error}",
            path.display(),
            backup.display()
        );
    }
}

fn secure_legacy_mcp_oauth_files_once() {
    static SECURED: OnceLock<()> = OnceLock::new();
    SECURED.get_or_init(|| {
        let path = mcp_oauth_path();
        for legacy in [path.clone(), path.with_extension("json.bak")] {
            if let Err(error) = secure_legacy_mcp_oauth_file(&legacy) {
                log::warn!("加密旧 MCP OAuth 文件失败 ({}): {error}", legacy.display());
            }
        }
    });
}

fn secure_legacy_mcp_oauth_file(path: &std::path::Path) -> Result<(), String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取失败: {error}")),
    };
    if content.starts_with(DPAPI_PREFIX) {
        return Ok(());
    }
    let (store, _) = decode_mcp_oauth_store_value(&content)?;
    let encrypted = encode_mcp_oauth_store_value(&store)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| format!("打开失败: {e}"))?;
    file.write_all(encrypted.as_bytes())
        .map_err(|e| format!("写入失败: {e}"))?;
    file.sync_all().map_err(|e| format!("同步失败: {e}"))
}

fn encode_mcp_oauth_store_value(store: &McpOAuthStore) -> Result<String, String> {
    let json = serde_json::to_vec(store).map_err(|e| format!("序列化 MCP OAuth 失败: {e}"))?;
    let encrypted = protect_mcp_oauth_data(&json)?;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    Ok(format!("{DPAPI_PREFIX}{}", STANDARD.encode(encrypted)))
}

fn decode_mcp_oauth_store_value(value: &str) -> Result<(McpOAuthStore, bool), String> {
    if let Some(encoded) = value.strip_prefix(DPAPI_PREFIX) {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let encrypted = STANDARD
            .decode(encoded)
            .map_err(|e| format!("解析 MCP OAuth DPAPI 数据失败: {e}"))?;
        let json = unprotect_mcp_oauth_data(&encrypted)?;
        let store = serde_json::from_slice(&json)
            .map_err(|e| format!("解析 MCP OAuth 密文内容失败: {e}"))?;
        Ok((store, false))
    } else {
        let store = serde_json::from_str(value)
            .map_err(|e| format!("解析 MCP OAuth 明文兼容数据失败: {e}"))?;
        Ok((store, true))
    }
}

#[cfg(windows)]
fn protect_mcp_oauth_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input_len = u32::try_from(data.len()).map_err(|_| "MCP OAuth 数据过大".to_string())?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("Windows DPAPI 加密失败: {e}"))?;
        if output.cbData > 0 && output.pbData.is_null() {
            return Err("Windows DPAPI 加密返回了无效内存".to_string());
        }
        let encrypted = if output.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
        };
        if !output.pbData.is_null() {
            let _ = LocalFree(HLOCAL(output.pbData.cast()));
        }
        Ok(encrypted)
    }
}

#[cfg(windows)]
fn unprotect_mcp_oauth_data(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input_len = u32::try_from(data.len()).map_err(|_| "MCP OAuth 密文过大".to_string())?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("Windows DPAPI 解密失败: {e}"))?;
        if output.cbData > 0 && output.pbData.is_null() {
            return Err("Windows DPAPI 解密返回了无效内存".to_string());
        }
        let plaintext = if output.cbData == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
        };
        if !output.pbData.is_null() {
            let _ = LocalFree(HLOCAL(output.pbData.cast()));
        }
        Ok(plaintext)
    }
}

#[cfg(not(windows))]
fn protect_mcp_oauth_data(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("MCP OAuth 安全持久化仅支持 Windows DPAPI".to_string())
}

#[cfg(not(windows))]
fn unprotect_mcp_oauth_data(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("MCP OAuth 安全持久化仅支持 Windows DPAPI".to_string())
}

fn normalize_mcp_oauth_store(store: &mut McpOAuthStore) -> bool {
    let mut changed = false;
    let mut credentials = std::mem::take(&mut store.creds_by_key);
    for (server_key, cred) in std::mem::take(&mut store.creds) {
        let credential_key = normalized_credential_key(&cred.mcp_endpoint);
        credentials.entry(credential_key.clone()).or_insert(cred);
        store
            .server_bindings
            .entry(binding_key("kiro", &server_key))
            .or_insert(credential_key);
        changed = true;
    }

    let mut remapped_keys = HashMap::new();
    let mut normalized_credentials: HashMap<String, McpOAuthCred> = HashMap::new();
    for (old_key, mut credential) in credentials {
        let canonical_endpoint = canonical_mcp_endpoint(&credential.mcp_endpoint);
        if canonical_endpoint != credential.mcp_endpoint {
            credential.mcp_endpoint = canonical_endpoint;
            changed = true;
        }
        let canonical_resource =
            canonical_oauth_resource(&credential.mcp_endpoint, &credential.resource);
        if canonical_resource != credential.resource {
            credential.resource = canonical_resource;
            changed = true;
        }
        let new_key = normalized_credential_key(&credential.mcp_endpoint);
        changed |= old_key != new_key;
        remapped_keys.insert(old_key, new_key.clone());
        match normalized_credentials.get(&new_key) {
            Some(existing)
                if existing.expires_at > credential.expires_at
                    || (existing.expires_at == credential.expires_at
                        && existing.refresh_token.is_some()
                        && credential.refresh_token.is_none()) => {}
            _ => {
                normalized_credentials.insert(new_key, credential);
            }
        }
    }
    store.creds_by_key = normalized_credentials;

    let existing_keys = store
        .creds_by_key
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let mut compatible_legacy_keys: HashMap<String, Option<String>> = HashMap::new();
    for (credential_key, credential) in &store.creds_by_key {
        let legacy_key = legacy_origin_credential_key(&credential.mcp_endpoint);
        compatible_legacy_keys
            .entry(legacy_key)
            .and_modify(|value| *value = None)
            .or_insert_with(|| Some(credential_key.clone()));
    }
    let find_compatible_key = |key: &str| {
        existing_keys
            .contains(key)
            .then(|| key.to_string())
            .or_else(|| compatible_legacy_keys.get(key).cloned().flatten())
    };

    for credential_key in store.server_bindings.values_mut() {
        let mapped = remapped_keys
            .get(credential_key)
            .cloned()
            .or_else(|| find_compatible_key(credential_key));
        if let Some(mapped) = mapped {
            if *credential_key != mapped {
                *credential_key = mapped;
                changed = true;
            }
        }
    }

    let old_failures = std::mem::take(&mut store.refresh_failures);
    let old_failures_snapshot = old_failures.clone();
    let mut normalized_failures = HashMap::new();
    for (credential_key, message) in old_failures {
        let mapped = remapped_keys
            .get(&credential_key)
            .cloned()
            .or_else(|| find_compatible_key(&credential_key))
            .unwrap_or(credential_key);
        normalized_failures.entry(mapped).or_insert(message);
    }
    changed |= normalized_failures != old_failures_snapshot;
    store.refresh_failures = normalized_failures;

    changed
}

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

fn encode_proxy_endpoint(endpoint: &str) -> String {
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

fn is_loopback_proxy_url(url: &url::Url) -> bool {
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

fn is_proxy_secret(value: &str) -> bool {
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

pub fn proxy_mcp_endpoint_from_url(value: &str, server_name: &str) -> Option<String> {
    let proxy = parse_mcp_oauth_proxy_url(value)?;
    if proxy.server_name != server_name {
        return None;
    }
    proxy.mcp_endpoint
}

fn endpoint_matches_server_name(endpoint: &str, server_name: &str) -> bool {
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

#[derive(Debug, Clone)]
pub enum McpOAuthConditionalUpdate {
    Applied(McpOAuthCred),
    Changed(McpOAuthCred),
    Missing,
}

#[derive(Clone)]
pub enum McpOAuthCredentialRollback {
    Restored(Box<McpOAuthCred>),
    Changed,
    Missing,
    InUse,
}

fn credential_matches(
    cred: &McpOAuthCred,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
) -> bool {
    cred.access_token == expected_access_token
        && cred.refresh_token.as_deref() == expected_refresh_token
}

pub fn upsert_mcp_oauth_cred(credential_key: &str, cred: McpOAuthCred) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    store.creds_by_key.insert(credential_key.to_string(), cred);
    store.refresh_failures.remove(credential_key);
    save_mcp_oauth_store_unlocked(&store)
}

fn rollback_credential_in_store_if_current(
    store: &mut McpOAuthStore,
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    previous: Option<McpOAuthCred>,
    previous_refresh_failure: Option<String>,
) -> McpOAuthCredentialRollback {
    let Some(current) = store.creds_by_key.get(credential_key).cloned() else {
        return McpOAuthCredentialRollback::Missing;
    };
    if !credential_matches(&current, expected_access_token, expected_refresh_token) {
        return McpOAuthCredentialRollback::Changed;
    }
    if previous.is_none()
        && store
            .server_bindings
            .values()
            .any(|value| value == credential_key)
    {
        return McpOAuthCredentialRollback::InUse;
    }

    match previous {
        Some(previous) => {
            store
                .creds_by_key
                .insert(credential_key.to_string(), previous);
        }
        None => {
            store.creds_by_key.remove(credential_key);
        }
    }
    match previous_refresh_failure {
        Some(message) => {
            store
                .refresh_failures
                .insert(credential_key.to_string(), message);
        }
        None => {
            store.refresh_failures.remove(credential_key);
        }
    }
    McpOAuthCredentialRollback::Restored(Box::new(current))
}

/// 授权提交失败时，仅在当前值仍是本次新签发 Token 的前提下恢复旧快照。
/// 返回被替换或删除的新凭据，供调用方异步撤销，避免覆盖并发刷新结果。
pub fn rollback_mcp_oauth_cred_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    previous: Option<McpOAuthCred>,
    previous_refresh_failure: Option<String>,
) -> Result<McpOAuthCredentialRollback, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let result = rollback_credential_in_store_if_current(
        &mut store,
        credential_key,
        expected_access_token,
        expected_refresh_token,
        previous,
        previous_refresh_failure,
    );
    if matches!(result, McpOAuthCredentialRollback::Restored(_)) {
        save_mcp_oauth_store_unlocked(&store)?;
    }
    Ok(result)
}

pub fn upsert_mcp_oauth_cred_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    updated: McpOAuthCred,
) -> Result<McpOAuthConditionalUpdate, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let current = store.creds_by_key.get(credential_key).cloned();
    match current {
        Some(current)
            if credential_matches(&current, expected_access_token, expected_refresh_token) =>
        {
            store
                .creds_by_key
                .insert(credential_key.to_string(), updated.clone());
            store.refresh_failures.remove(credential_key);
            save_mcp_oauth_store_unlocked(&store)?;
            Ok(McpOAuthConditionalUpdate::Applied(updated))
        }
        Some(current) => Ok(McpOAuthConditionalUpdate::Changed(current)),
        None => Ok(McpOAuthConditionalUpdate::Missing),
    }
}

pub fn set_mcp_oauth_refresh_failure_if_current(
    credential_key: &str,
    expected_access_token: &str,
    expected_refresh_token: Option<&str>,
    message: String,
) -> Result<McpOAuthConditionalUpdate, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let current = store.creds_by_key.get(credential_key).cloned();
    match current {
        Some(current)
            if credential_matches(&current, expected_access_token, expected_refresh_token) =>
        {
            let mut current = current;
            current.last_refresh_attempt_at = now_secs();
            store
                .creds_by_key
                .insert(credential_key.to_string(), current.clone());
            store
                .refresh_failures
                .insert(credential_key.to_string(), message);
            save_mcp_oauth_store_unlocked(&store)?;
            Ok(McpOAuthConditionalUpdate::Applied(current))
        }
        Some(current) => Ok(McpOAuthConditionalUpdate::Changed(current)),
        None => Ok(McpOAuthConditionalUpdate::Missing),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn bind_mcp_oauth_server(
    client: &str,
    server_name: &str,
    credential_key: &str,
) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    store
        .server_bindings
        .insert(binding_key(client, server_name), credential_key.to_string());
    save_mcp_oauth_store_unlocked(&store)
}

/// 原子替换（或清除）一个客户端绑定，并回收不再被任何客户端引用的旧凭据。
/// 返回被回收的旧凭据，供调用方在本地提交成功后尽力通知远端撤销。
pub fn replace_mcp_oauth_binding(
    client: &str,
    server_name: &str,
    credential_key: Option<&str>,
) -> Result<Option<(String, McpOAuthCred)>, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    if let Some(credential_key) = credential_key {
        if !store.creds_by_key.contains_key(credential_key) {
            return Err("要绑定的 MCP OAuth 凭据不存在".to_string());
        }
    }

    let binding = binding_key(client, server_name);
    let previous = store.server_bindings.get(&binding).cloned();
    if previous.as_deref() == credential_key {
        return Ok(None);
    }

    match credential_key {
        Some(credential_key) => {
            store
                .server_bindings
                .insert(binding, credential_key.to_string());
        }
        None => {
            store.server_bindings.remove(&binding);
        }
    }

    let removed = previous.and_then(|previous_key| {
        let still_used = store
            .server_bindings
            .values()
            .any(|value| value == &previous_key);
        if still_used {
            return None;
        }
        store.refresh_failures.remove(&previous_key);
        store
            .creds_by_key
            .remove(&previous_key)
            .map(|credential| (previous_key, credential))
    });
    save_mcp_oauth_store_unlocked(&store)?;
    Ok(removed)
}

/// 仅在凭据没有任何绑定时删除，用于授权提交失败后的补偿清理。
pub fn remove_mcp_oauth_cred_if_unbound(
    credential_key: &str,
) -> Result<Option<McpOAuthCred>, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    if store
        .server_bindings
        .values()
        .any(|value| value == credential_key)
    {
        return Ok(None);
    }
    store.refresh_failures.remove(credential_key);
    let removed = store.creds_by_key.remove(credential_key);
    if removed.is_some() {
        save_mcp_oauth_store_unlocked(&store)?;
    }
    Ok(removed)
}

pub fn get_mcp_oauth_binding(client: &str, server_name: &str) -> Result<Option<String>, String> {
    let store = get_mcp_oauth_store()?;
    Ok(store
        .server_bindings
        .get(&binding_key(client, server_name))
        .cloned())
}

/// 读取绑定及其共享状态，用于在服务端撤销前判断该凭据是否仍被其他
/// 客户端使用。返回值为 `(credential_key, credential, removed_last)`。
pub fn get_mcp_oauth_binding_details(
    client: &str,
    server_name: &str,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    let store = get_mcp_oauth_store()?;
    let binding = binding_key(client, server_name);
    let Some(credential_key) = store.server_bindings.get(&binding).cloned() else {
        return Ok(None);
    };
    let Some(cred) = store.creds_by_key.get(&credential_key).cloned() else {
        return Ok(None);
    };
    let still_used = store
        .server_bindings
        .iter()
        .any(|(key, value)| key != &binding && value == &credential_key);
    Ok(Some((credential_key, cred, !still_used)))
}

pub fn unbind_mcp_oauth_server(
    client: &str,
    server_name: &str,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    unbind_mcp_oauth_server_if_current(client, server_name, None)
}

/// 与预读绑定组合成条件删除，避免撤销请求等待网络期间其它客户端
/// 刚好改绑后误删/误撤销新的凭据。
pub fn unbind_mcp_oauth_server_if_current(
    client: &str,
    server_name: &str,
    expected_credential_key: Option<&str>,
) -> Result<Option<(String, McpOAuthCred, bool)>, String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let binding = binding_key(client, server_name);
    let Some(credential_key) = store.server_bindings.get(&binding).cloned() else {
        save_mcp_oauth_store_unlocked(&store)?;
        return Ok(None);
    };
    if expected_credential_key.is_some_and(|expected| expected != credential_key) {
        return Err("MCP OAuth 绑定在撤销期间发生变化，请重新操作".to_string());
    }
    store.server_bindings.remove(&binding);

    let still_used = store.server_bindings.values().any(|v| v == &credential_key);
    let cred = store.creds_by_key.get(&credential_key).cloned();
    let removed_last = !still_used;

    if removed_last {
        store.creds_by_key.remove(&credential_key);
        store.refresh_failures.remove(&credential_key);
    }

    save_mcp_oauth_store_unlocked(&store)?;
    Ok(cred.map(|c| (credential_key, c, removed_last)))
}

pub fn get_or_init_proxy_runtime() -> Result<(u16, String), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    let mut changed = false;

    if store.proxy_port.is_none() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("分配反代端口失败: {e}"))?;
        store.proxy_port = Some(
            listener
                .local_addr()
                .map_err(|e| format!("获取端口失败: {e}"))?
                .port(),
        );
        changed = true;
    }

    if store.proxy_secret.is_none() {
        store.proxy_secret = Some(uuid::Uuid::new_v4().simple().to_string());
        changed = true;
    }

    if changed {
        save_mcp_oauth_store_unlocked(&store)?;
    }

    let port = store
        .proxy_port
        .ok_or_else(|| "MCP OAuth 反代端口初始化失败".to_string())?;
    let secret = store
        .proxy_secret
        .ok_or_else(|| "MCP OAuth 反代密钥初始化失败".to_string())?;

    Ok((port, secret))
}

pub fn set_proxy_runtime_port(port: u16) -> Result<(), String> {
    let _guard = lock_mcp_oauth_store()?;
    let mut store = load_mcp_oauth_store_unlocked()?;
    if store.proxy_port != Some(port) {
        store.proxy_port = Some(port);
        save_mcp_oauth_store_unlocked(&store)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}

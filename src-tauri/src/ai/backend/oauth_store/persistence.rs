use super::*;

pub(super) fn mcp_oauth_path() -> PathBuf {
    data_dir().join("mcp-oauth.json")
}

pub(super) fn mcp_oauth_store_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn lock_mcp_oauth_store() -> Result<MutexGuard<'static, ()>, String> {
    mcp_oauth_store_lock()
        .lock()
        .map_err(|_| "MCP OAuth 凭据存储锁已损坏".to_string())
}

pub(super) fn load_mcp_oauth_store_unlocked() -> Result<McpOAuthStore, String> {
    if let Some(store) = oauth_cache().get() {
        return Ok((*store).clone());
    }
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

    oauth_cache().publish(store.clone());
    Ok(store)
}

pub(super) fn save_mcp_oauth_store_unlocked(store: &McpOAuthStore) -> Result<(), String> {
    let content = encode_mcp_oauth_store_value(store)?;
    oauth_cache().commit(store, || {
        crate::ai::backend::db::kv_set("mcp_oauth", "store", &content)
    })
}

pub fn get_mcp_oauth_store() -> Result<Arc<McpOAuthStore>, String> {
    if let Some(store) = oauth_cache().get() {
        return Ok(store);
    }
    let _guard = lock_mcp_oauth_store()?;
    load_mcp_oauth_store_unlocked()?;
    oauth_cache()
        .get()
        .ok_or_else(|| "OAuth 快照初始化失败".to_string())
}

pub(super) fn load_legacy_mcp_oauth() -> Result<Option<McpOAuthStore>, String> {
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

pub(super) fn archive_legacy_mcp_oauth() {
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

pub(super) fn secure_legacy_mcp_oauth_files_once() {
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

pub(super) fn secure_legacy_mcp_oauth_file(path: &std::path::Path) -> Result<(), String> {
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

pub(super) fn encode_mcp_oauth_store_value(store: &McpOAuthStore) -> Result<String, String> {
    let json = serde_json::to_vec(store).map_err(|e| format!("序列化 MCP OAuth 失败: {e}"))?;
    let encrypted = protect_mcp_oauth_data(&json)?;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    Ok(format!("{DPAPI_PREFIX}{}", STANDARD.encode(encrypted)))
}

pub(super) fn decode_mcp_oauth_store_value(value: &str) -> Result<(McpOAuthStore, bool), String> {
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
pub(super) fn protect_mcp_oauth_data(data: &[u8]) -> Result<Vec<u8>, String> {
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
pub(super) fn unprotect_mcp_oauth_data(data: &[u8]) -> Result<Vec<u8>, String> {
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
pub(super) fn protect_mcp_oauth_data(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("MCP OAuth 安全持久化仅支持 Windows DPAPI".to_string())
}

#[cfg(not(windows))]
pub(super) fn unprotect_mcp_oauth_data(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("MCP OAuth 安全持久化仅支持 Windows DPAPI".to_string())
}

pub(super) fn normalize_mcp_oauth_store(store: &mut McpOAuthStore) -> bool {
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

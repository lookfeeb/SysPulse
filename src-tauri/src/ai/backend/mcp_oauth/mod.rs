// 远程 MCP 服务器 OAuth (RFC 8414 发现 + RFC 7591 DCR + PKCE 授权码)
// 用于在本应用内为 url 型 MCP 服务器（如 Notion）完成授权，
// Token 存入 SysPulse AI 管理凭据存储，由本地反代注入 Bearer。

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::ai::backend::oauth_store::{
    canonical_oauth_resource, get_mcp_oauth_store, mcp_oauth_failure_needs_reauth,
    set_mcp_oauth_refresh_failure_if_current, upsert_mcp_oauth_cred_if_current,
    McpOAuthConditionalUpdate, McpOAuthCred,
};
use crate::ai::backend::utils::browser::open_browser_keep_session;

/// 授权流程产出：换到 token + 端点信息，供命令层落盘
pub struct AuthorizeOutcome {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    pub resource: String,
}

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub resource: String,
}

#[derive(Debug, Clone)]
pub enum OAuthProbeResult {
    Supported(Endpoints),
    NotAdvertised,
}

#[derive(Deserialize)]
struct MetadataResp {
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    registration_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct ProtectedResourceMetadataResp {
    resource: String,
    #[serde(default)]
    authorization_servers: Vec<String>,
}

#[derive(Clone)]
struct CachedOAuthProbe {
    checked_at: Instant,
    result: Result<OAuthProbeResult, String>,
}

#[derive(Deserialize)]
struct RegisterResp {
    client_id: String,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

mod http;
use http::*;
mod discovery;
pub use discovery::*;
mod tokens;
pub use tokens::*;
mod authorization;
pub use authorization::*;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpServerItem {
    pub name: String,
    pub client: String,
    #[serde(rename = "type")]
    pub server_type: String,
    pub detail: String,
    pub disabled: bool,
}

impl From<crate::ai::backend::commands::mcp_cmd::McpServerItem> for AiMcpServerItem {
    fn from(value: crate::ai::backend::commands::mcp_cmd::McpServerItem) -> Self {
        let detail = redact_mcp_detail(&value.detail, &value.server_type);
        Self {
            name: value.name,
            client: value.client,
            server_type: value.server_type,
            detail,
            disabled: value.disabled,
        }
    }
}

fn redact_mcp_detail(detail: &str, server_type: &str) -> String {
    if !matches!(server_type, "url" | "http" | "sse") {
        return detail.to_string();
    }
    let visible = crate::ai::backend::oauth_store::parse_mcp_oauth_proxy_url(detail)
        .and_then(|proxy| proxy.mcp_endpoint)
        .unwrap_or_else(|| detail.to_string());
    let Ok(mut url) = url::Url::parse(&visible) else {
        return if crate::ai::backend::oauth_store::is_mcp_oauth_proxy_url(detail) {
            "OAuth 本地代理（真实地址已隐藏）".to_string()
        } else {
            visible
        };
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpClientStats {
    pub client: String,
    pub total_servers: usize,
    pub enabled_servers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpOverview {
    pub total_servers: usize,
    pub enabled_servers: usize,
    pub clients: Vec<AiMcpClientStats>,
}

impl From<crate::ai::backend::commands::mcp_cmd::McpClientsOverview> for AiMcpOverview {
    fn from(value: crate::ai::backend::commands::mcp_cmd::McpClientsOverview) -> Self {
        Self {
            total_servers: value.total_servers,
            enabled_servers: value.enabled_servers,
            clients: value
                .clients
                .into_iter()
                .map(|client| AiMcpClientStats {
                    client: client.client,
                    total_servers: client.total_servers,
                    enabled_servers: client.enabled_servers,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMcpOAuthStatus {
    pub oauth_supported: Option<bool>,
    pub authorized: bool,
    pub expires_at: i64,
    pub expiring_soon: bool,
    pub expired: bool,
    pub refresh_failed: bool,
    pub needs_reauth: bool,
    pub message: Option<String>,
}

impl From<crate::ai::backend::commands::mcp_oauth_cmd::McpOAuthStatus> for AiMcpOAuthStatus {
    fn from(value: crate::ai::backend::commands::mcp_oauth_cmd::McpOAuthStatus) -> Self {
        Self {
            oauth_supported: value.oauth_supported,
            authorized: value.authorized,
            expires_at: value.expires_at,
            expiring_soon: value.expiring_soon,
            expired: value.expired,
            refresh_failed: value.refresh_failed,
            needs_reauth: value.needs_reauth,
            message: value.message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionSummary {
    pub session_id: String,
    pub title: String,
    pub session_type: String,
    pub workspace_directory: String,
    pub workspace_hash: String,
    pub message_count: usize,
    pub file_size: u64,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub source: String,
}

impl From<crate::ai::backend::models::ide_session::SessionSummary> for AiSessionSummary {
    fn from(value: crate::ai::backend::models::ide_session::SessionSummary) -> Self {
        Self {
            session_id: value.session_id,
            title: value.title,
            session_type: value.session_type,
            workspace_directory: value.workspace_directory,
            workspace_hash: value.workspace_hash,
            message_count: value.message_count,
            file_size: value.file_size,
            created_at: value.created_at,
            modified_at: value.modified_at,
            source: value.source,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionTree {
    pub workspaces: Vec<String>,
    pub sessions_by_workspace: HashMap<String, Vec<AiSessionSummary>>,
}

impl From<crate::ai::backend::models::ide_session::SessionTree> for AiSessionTree {
    fn from(value: crate::ai::backend::models::ide_session::SessionTree) -> Self {
        Self {
            workspaces: value.workspaces,
            sessions_by_workspace: value
                .sessions_by_workspace
                .into_iter()
                .map(|(workspace, sessions)| {
                    (
                        workspace,
                        sessions.into_iter().map(AiSessionSummary::from).collect(),
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiContentItem {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiMessage {
    pub role: String,
    pub content: Vec<AiContentItem>,
    pub id: String,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiHistoryItem {
    pub message: AiMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSession {
    pub session_id: String,
    pub title: String,
    pub session_type: String,
    pub workspace_directory: String,
    pub history: Vec<AiHistoryItem>,
    pub conversation_summary: Option<String>,
}

impl From<crate::ai::backend::models::ide_session::IdeSession> for AiSession {
    fn from(value: crate::ai::backend::models::ide_session::IdeSession) -> Self {
        Self {
            session_id: value.session_id,
            title: value.title,
            session_type: value.session_type,
            workspace_directory: value.workspace_directory,
            history: value
                .history
                .into_iter()
                .map(|item| AiHistoryItem {
                    message: AiMessage {
                        role: item.message.role,
                        content: item
                            .message
                            .content
                            .into_iter()
                            .map(|content| AiContentItem {
                                content_type: content.content_type,
                                text: content.text,
                            })
                            .collect(),
                        id: item.message.id,
                        is_hidden: item.message.is_hidden,
                    },
                })
                .collect(),
            conversation_summary: value.conversation_summary,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionPage {
    pub session_id: String,
    pub title: String,
    pub session_type: String,
    pub workspace_directory: String,
    pub history: Vec<AiHistoryItem>,
    pub conversation_summary: Option<String>,
    pub total_messages: usize,
    pub page: usize,
    pub page_size: usize,
}

impl From<crate::ai::backend::models::ide_session::SessionPage> for AiSessionPage {
    fn from(value: crate::ai::backend::models::ide_session::SessionPage) -> Self {
        Self {
            session_id: value.session_id,
            title: value.title,
            session_type: value.session_type,
            workspace_directory: value.workspace_directory,
            history: value
                .history
                .into_iter()
                .map(|item| AiHistoryItem {
                    message: AiMessage {
                        role: item.message.role,
                        content: item
                            .message
                            .content
                            .into_iter()
                            .map(|content| AiContentItem {
                                content_type: content.content_type,
                                text: content.text,
                            })
                            .collect(),
                        id: item.message.id,
                        is_hidden: item.message.is_hidden,
                    },
                })
                .collect(),
            conversation_summary: value.conversation_summary,
            total_messages: value.total_messages,
            page: value.page,
            page_size: value.page_size,
        }
    }
}

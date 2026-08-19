use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerItem {
    pub name: String,
    pub client: String,
    #[serde(rename = "type")]
    pub server_type: String,
    pub detail: String,
    pub disabled: bool,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientStats {
    pub client: String,
    pub total_servers: usize,
    pub enabled_servers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClientsOverview {
    pub total_servers: usize,
    pub enabled_servers: usize,
    pub clients: Vec<McpClientStats>,
}

// MCP 配置文件读写

use serde::{Deserialize, Serialize};

use crate::ai::backend::utils::fs::atomic_write;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub powers: Option<PowersMcpConfig>,
    /// 保留 Kiro 后续版本或插件写入的未知根字段，避免 SysPulse 保存时丢失。
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PowersMcpConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, PowerMcpServer>,
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServer {
    Command(McpServerCommand),
    Url(McpServerUrl),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, rename = "autoApprove")]
    pub auto_approve: Vec<String>,
    /// 原样保留客户端私有扩展（如超时、请求头或运行参数）。
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerUrl {
    pub url: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, rename = "disabledTools")]
    pub disabled_tools: Vec<String>,
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerMcpServer {
    pub url: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default, rename = "disabledTools")]
    pub disabled_tools: Vec<String>,
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl McpConfig {
    /// 获取用户级 MCP 配置文件路径
    pub fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".kiro").join("settings").join("mcp.json"))
    }

    /// 获取项目级 MCP 配置文件路径
    pub fn project_config_path(project_dir: &str) -> PathBuf {
        PathBuf::from(project_dir)
            .join(".kiro")
            .join("settings")
            .join("mcp.json")
    }

    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(|e| format!("读取配置文件失败: {e}"))?;

        serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {e}"))
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }

        let content =
            serde_json::to_string_pretty(self).map_err(|e| format!("序列化配置失败: {e}"))?;

        atomic_write(path, &content, "MCP 配置文件")
    }

    /// 读取用户级配置（写操作使用）
    pub fn load() -> Result<Self, String> {
        let path = Self::config_path().ok_or("无法获取用户目录")?;
        Self::load_from_path(&path)
    }

    /// 读取有效配置：user -> workspace -> powers（后者覆盖前者）
    pub fn load_merged(project_dir: Option<&str>) -> Result<Self, String> {
        let user_path = Self::config_path().ok_or("无法获取用户目录")?;
        let user_config = Self::load_from_path(&user_path)?;

        let workspace_config = if let Some(pd) = project_dir {
            let ws_path = Self::project_config_path(pd);
            Self::load_from_path(&ws_path)?
        } else {
            Self::default()
        };

        let mut merged_servers = user_config.mcp_servers.clone();
        for (name, server) in workspace_config.mcp_servers {
            merged_servers.insert(name, server);
        }

        if let Some(powers) = user_config.powers.as_ref() {
            for (name, server) in &powers.mcp_servers {
                merged_servers.insert(
                    name.clone(),
                    McpServer::Url(McpServerUrl {
                        url: server.url.clone(),
                        disabled: server.disabled,
                        disabled_tools: server.disabled_tools.clone(),
                        extra: server.extra.clone(),
                    }),
                );
            }
        }

        Ok(Self {
            mcp_servers: merged_servers,
            powers: user_config.powers,
            extra: user_config.extra,
        })
    }

    /// 保存用户级配置文件
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("无法获取用户目录")?;
        self.save_to_path(&path)
    }
}

impl From<PowerMcpServer> for McpServerUrl {
    fn from(value: PowerMcpServer) -> Self {
        Self {
            url: value.url,
            disabled: value.disabled,
            disabled_tools: value.disabled_tools,
            extra: value.extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kiro_save_preserves_unknown_root_power_and_server_fields() {
        let dir = std::env::temp_dir().join(format!("syspulse-kiro-mcp-{}", uuid::Uuid::new_v4()));
        let path = dir.join("mcp.json");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "futureRoot": { "enabled": true },
                "mcpServers": {
                    "demo": {
                        "command": "node",
                        "args": ["server.js"],
                        "disabled": false,
                        "timeoutMs": 30000,
                        "transportOptions": { "stdio": true }
                    }
                },
                "powers": {
                    "futurePower": "keep",
                    "mcpServers": {
                        "remote": {
                            "url": "https://example.com/mcp",
                            "headers": { "X-Custom": "value" }
                        }
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let mut config = McpConfig::load_from_path(&path).unwrap();
        match config.mcp_servers.get_mut("demo").unwrap() {
            McpServer::Command(server) => server.disabled = true,
            McpServer::Url(_) => panic!("测试配置应解析为 command"),
        }
        config.save_to_path(&path).unwrap();

        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["futureRoot"]["enabled"], true);
        assert_eq!(saved["mcpServers"]["demo"]["disabled"], true);
        assert_eq!(saved["mcpServers"]["demo"]["timeoutMs"], 30000);
        assert_eq!(
            saved["mcpServers"]["demo"]["transportOptions"]["stdio"],
            true
        );
        assert_eq!(saved["powers"]["futurePower"], "keep");
        assert_eq!(
            saved["powers"]["mcpServers"]["remote"]["headers"]["X-Custom"],
            "value"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

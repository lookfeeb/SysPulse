use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::ai::backend::kiro::settings::mcp::{McpConfig, McpServer};
use crate::ai::backend::utils::fs::atomic_write;

use super::types::McpServerItem;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum McpClientKind {
    Kiro,
    Codex,
    ClaudeCli,
}

impl McpClientKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "kiro" => Ok(Self::Kiro),
            "codex" => Ok(Self::Codex),
            "claude-cli" | "claude_cli" | "claude" => Ok(Self::ClaudeCli),
            _ => Err(format!("不支持的 MCP 客户端: {value}")),
        }
    }

    pub fn as_key(self) -> &'static str {
        match self {
            Self::Kiro => "kiro",
            Self::Codex => "codex",
            Self::ClaudeCli => "claude-cli",
        }
    }
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or("无法获取用户目录".to_string())
}

fn mcp_config_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_mcp_config_writes() -> Result<MutexGuard<'static, ()>, String> {
    mcp_config_write_lock()
        .lock()
        .map_err(|_| "MCP 客户端配置写入锁已损坏".to_string())
}

/// 所有 MCP 配置变更共用同一协调器。调用方必须在闭包内重新读取最新文件，
/// 禁止把锁外读取的旧快照直接写回。
pub(super) fn with_mcp_config_write_lock<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guard = lock_mcp_config_writes()?;
    operation()
}

fn codex_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn claude_cli_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude.json"))
}

fn server_item_from_json(
    name: &str,
    client: McpClientKind,
    raw: serde_json::Value,
) -> McpServerItem {
    let url = raw.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    let command = raw
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let raw_type = raw.get("type").and_then(|v| v.as_str()).unwrap_or_default();
    let server_type = if !url.is_empty()
        || matches!(
            raw_type.to_ascii_lowercase().as_str(),
            "http" | "sse" | "url"
        ) {
        if raw_type.is_empty() { "url" } else { raw_type }.to_ascii_lowercase()
    } else {
        "command".to_string()
    };

    McpServerItem {
        name: name.to_string(),
        client: client.as_key().to_string(),
        server_type,
        detail: if !url.is_empty() {
            url.to_string()
        } else {
            command.to_string()
        },
        disabled: raw
            .get("disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        raw,
    }
}

fn kiro_server_to_raw(server: &McpServer) -> serde_json::Value {
    serde_json::to_value(server).unwrap_or_else(|_| serde_json::json!({}))
}

fn raw_to_kiro_server(raw: serde_json::Value) -> Result<McpServer, String> {
    serde_json::from_value(raw).map_err(|e| format!("转换 Kiro MCP 配置失败: {e}"))
}

fn load_kiro_items() -> Result<Vec<McpServerItem>, String> {
    let config = McpConfig::load()?;
    let mut items: Vec<_> = config
        .mcp_servers
        .into_iter()
        .map(|(name, server)| {
            server_item_from_json(&name, McpClientKind::Kiro, kiro_server_to_raw(&server))
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

fn load_codex_doc() -> Result<(PathBuf, DocumentMut), String> {
    let path = codex_config_path()?;
    let content = if path.exists() {
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex config.toml 失败: {e}"))?
    } else {
        String::new()
    };
    let doc = content
        .parse::<DocumentMut>()
        .map_err(|e| format!("解析 Codex config.toml 失败: {e}"))?;
    Ok((path, doc))
}

fn save_codex_doc(path: &Path, doc: &DocumentMut) -> Result<(), String> {
    atomic_write(path, &doc.to_string(), "Codex config.toml")
}

fn codex_server_table_mut<'a>(
    doc: &'a mut DocumentMut,
    server_name: &str,
) -> Result<&'a mut Table, String> {
    doc.get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .and_then(|servers| servers.get_mut(server_name))
        .and_then(Item::as_table_mut)
        .ok_or_else(|| format!("codex 中不存在 MCP 服务器 {server_name}"))
}

fn toml_value_to_json(item: &Item) -> serde_json::Value {
    if let Some(value) = item.as_value() {
        if let Some(s) = value.as_str() {
            return serde_json::Value::String(s.to_string());
        }
        if let Some(b) = value.as_bool() {
            return serde_json::Value::Bool(b);
        }
        if let Some(i) = value.as_integer() {
            return serde_json::Value::Number(i.into());
        }
        if let Some(f) = value.as_float() {
            return serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null);
        }
        if let Some(datetime) = value.as_datetime() {
            return serde_json::Value::String(datetime.to_string());
        }
        if let Some(arr) = value.as_array() {
            return serde_json::Value::Array(arr.iter().map(toml_edit_value_to_json).collect());
        }
        if let Some(table) = value.as_inline_table() {
            return serde_json::Value::Object(
                table
                    .iter()
                    .map(|(key, value)| (key.to_string(), toml_edit_value_to_json(value)))
                    .collect(),
            );
        }
    }
    if let Some(table) = item.as_table() {
        let mut map = serde_json::Map::new();
        for (key, value) in table.iter() {
            map.insert(key.to_string(), toml_value_to_json(value));
        }
        return serde_json::Value::Object(map);
    }
    if let Some(tables) = item.as_array_of_tables() {
        return serde_json::Value::Array(
            tables
                .iter()
                .map(|table| toml_value_to_json(&Item::Table(table.clone())))
                .collect(),
        );
    }
    serde_json::Value::Null
}

fn toml_edit_value_to_json(value: &Value) -> serde_json::Value {
    toml_value_to_json(&Item::Value(value.clone()))
}

fn load_codex_items() -> Result<Vec<McpServerItem>, String> {
    let (_, doc) = load_codex_doc()?;
    let mut items = Vec::new();
    if let Some(table) = doc.get("mcp_servers").and_then(|i| i.as_table()) {
        for (name, item) in table.iter() {
            items.push(server_item_from_json(
                name,
                McpClientKind::Codex,
                toml_value_to_json(item),
            ));
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

fn toml_value_from_json(raw: &serde_json::Value) -> Option<Value> {
    match raw {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(Value::from(value.clone())),
        serde_json::Value::Bool(value) => Some(Value::from(*value)),
        serde_json::Value::Number(value) => value
            .as_i64()
            .map(Value::from)
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|n| i64::try_from(n).ok())
                    .map(Value::from)
            })
            .or_else(|| value.as_f64().map(Value::from)),
        serde_json::Value::Array(values) => {
            let mut array = Array::new();
            for value in values {
                if let Some(value) = toml_value_from_json(value) {
                    array.push(value);
                }
            }
            Some(Value::Array(array))
        }
        serde_json::Value::Object(values) => {
            let mut table = InlineTable::new();
            for (key, value) in values {
                if let Some(value) = toml_value_from_json(value) {
                    table.insert(key, value);
                }
            }
            Some(Value::InlineTable(table))
        }
    }
}

fn table_from_json(raw: &serde_json::Value) -> Table {
    let mut table = Table::new();
    if let Some(obj) = raw.as_object() {
        for (key, val) in obj {
            if let Some(value) = toml_value_from_json(val) {
                table.insert(key, Item::Value(value));
            }
        }
    }
    table
}

fn load_claude_root() -> Result<(PathBuf, serde_json::Value), String> {
    let path = claude_cli_config_path()?;
    if !path.exists() {
        return Ok((path, serde_json::json!({ "mcpServers": {} })));
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Claude CLI 配置失败: {e}"))?;
    let value =
        serde_json::from_str(&content).map_err(|e| format!("解析 Claude CLI 配置失败: {e}"))?;
    Ok((path, value))
}

fn save_claude_root(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败: {e}"))?;
    atomic_write(path, &content, "Claude CLI 配置")
}

fn claude_server_object_mut<'a>(
    root: &'a mut serde_json::Value,
    server_name: &str,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, String> {
    root.get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|servers| servers.get_mut(server_name))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| format!("claude-cli 中不存在 MCP 服务器 {server_name}"))
}

fn load_claude_items() -> Result<Vec<McpServerItem>, String> {
    let (_, root) = load_claude_root()?;
    let mut items = Vec::new();
    if let Some(servers) = root.get("mcpServers").and_then(|v| v.as_object()) {
        for (name, raw) in servers {
            items.push(server_item_from_json(
                name,
                McpClientKind::ClaudeCli,
                raw.clone(),
            ));
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

pub fn load_mcp_items_for_client(client: McpClientKind) -> Result<Vec<McpServerItem>, String> {
    match client {
        McpClientKind::Kiro => load_kiro_items(),
        McpClientKind::Codex => load_codex_items(),
        McpClientKind::ClaudeCli => load_claude_items(),
    }
}

pub fn read_mcp_server_for_client(
    client: &str,
    server_name: &str,
) -> Result<McpServerItem, String> {
    let client = McpClientKind::parse(client)?;
    load_mcp_items_for_client(client)?
        .into_iter()
        .find(|s| s.name == server_name)
        .ok_or_else(|| format!("{} 中不存在 MCP 服务器 {server_name}", client.as_key()))
}

pub fn read_mcp_server_url_for_client(client: &str, server_name: &str) -> Result<String, String> {
    let item = read_mcp_server_for_client(client, server_name)?;
    if matches!(item.server_type.as_str(), "url" | "http" | "sse") {
        item.raw
            .get("url")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .ok_or_else(|| "该服务器没有 url 字段".to_string())
    } else {
        Err("command 型服务器不支持 OAuth".to_string())
    }
}

/// 在锁内检查覆盖条件并安装配置，返回目标位置原来的原始配置。
pub fn install_mcp_server_for_client(
    client: &str,
    server_name: &str,
    raw: serde_json::Value,
    overwrite: bool,
) -> Result<Option<serde_json::Value>, String> {
    with_mcp_config_write_lock(|| {
        let previous = match read_mcp_server_for_client(client, server_name) {
            Ok(item) => Some(item.raw),
            Err(error) if error.contains("不存在 MCP 服务器") => None,
            Err(error) => return Err(error),
        };
        if previous.is_some() && !overwrite {
            return Err(format!("{client} 已存在 MCP 服务器 {server_name}"));
        }
        write_mcp_server_for_client_unlocked(client, server_name, raw)?;
        Ok(previous)
    })
}

pub fn restore_mcp_server_for_client(
    client: &str,
    server_name: &str,
    previous: Option<serde_json::Value>,
) -> Result<(), String> {
    with_mcp_config_write_lock(|| match previous {
        Some(raw) => write_mcp_server_for_client_unlocked(client, server_name, raw),
        None => delete_mcp_server_for_client_unlocked(client, server_name),
    })
}

fn write_mcp_server_for_client_unlocked(
    client: &str,
    server_name: &str,
    raw: serde_json::Value,
) -> Result<(), String> {
    match McpClientKind::parse(client)? {
        McpClientKind::Kiro => {
            let mut config = McpConfig::load()?;
            config
                .mcp_servers
                .insert(server_name.to_string(), raw_to_kiro_server(raw)?);
            config.save()
        }
        McpClientKind::Codex => {
            let (path, mut doc) = load_codex_doc()?;
            let root = doc.as_table_mut();
            let servers = root
                .entry("mcp_servers")
                .or_insert_with(|| Item::Table(Table::new()))
                .as_table_mut()
                .ok_or("Codex mcp_servers 不是 table")?;
            servers.insert(server_name, Item::Table(table_from_json(&raw)));
            save_codex_doc(&path, &doc)
        }
        McpClientKind::ClaudeCli => {
            let (path, mut root) = load_claude_root()?;
            if !root.is_object() {
                root = serde_json::json!({});
            }
            let obj = root
                .as_object_mut()
                .ok_or_else(|| "Claude MCP 配置根节点不是对象".to_string())?;
            let servers = obj
                .entry("mcpServers".to_string())
                .or_insert_with(|| serde_json::json!({}));
            if !servers.is_object() {
                *servers = serde_json::json!({});
            }
            servers
                .as_object_mut()
                .ok_or_else(|| "Claude mcpServers 节点不是对象".to_string())?
                .insert(server_name.to_string(), raw);
            save_claude_root(&path, &root)
        }
    }
}

pub fn write_mcp_server_url_for_client(
    client: &str,
    server_name: &str,
    new_url: &str,
) -> Result<(), String> {
    with_mcp_config_write_lock(|| {
        write_mcp_server_url_for_client_unlocked(client, server_name, new_url)
    })
}

fn write_mcp_server_url_for_client_unlocked(
    client: &str,
    server_name: &str,
    new_url: &str,
) -> Result<(), String> {
    match McpClientKind::parse(client)? {
        McpClientKind::Kiro => {
            let mut config = McpConfig::load()?;
            let server = config
                .mcp_servers
                .get_mut(server_name)
                .ok_or_else(|| format!("kiro 中不存在 MCP 服务器 {server_name}"))?;
            match server {
                McpServer::Url(server) => server.url = new_url.to_string(),
                McpServer::Command(_) => return Err("command 型服务器不支持 OAuth".to_string()),
            }
            config.save()
        }
        McpClientKind::Codex => {
            let (path, mut doc) = load_codex_doc()?;
            let server = codex_server_table_mut(&mut doc, server_name)?;
            if server.get("url").is_none() {
                return Err("command 型服务器不支持 OAuth".to_string());
            }
            server.insert("url", value(new_url));
            save_codex_doc(&path, &doc)
        }
        McpClientKind::ClaudeCli => {
            let (path, mut root) = load_claude_root()?;
            let server = claude_server_object_mut(&mut root, server_name)?;
            if !server.get("url").is_some_and(serde_json::Value::is_string) {
                return Err("command 型服务器不支持 OAuth".to_string());
            }
            server.insert(
                "url".to_string(),
                serde_json::Value::String(new_url.to_string()),
            );
            save_claude_root(&path, &root)
        }
    }
}

pub fn write_mcp_server_url_for_client_if_current(
    client: &str,
    server_name: &str,
    expected_url: &str,
    new_url: &str,
) -> Result<bool, String> {
    with_mcp_config_write_lock(|| {
        let current = read_mcp_server_url_for_client(client, server_name)?;
        if current != expected_url {
            return Ok(false);
        }
        write_mcp_server_url_for_client_unlocked(client, server_name, new_url)?;
        Ok(true)
    })
}

pub fn delete_mcp_server_for_client(client: &str, name: &str) -> Result<(), String> {
    with_mcp_config_write_lock(|| delete_mcp_server_for_client_unlocked(client, name))
}

fn delete_mcp_server_for_client_unlocked(client: &str, name: &str) -> Result<(), String> {
    match McpClientKind::parse(client)? {
        McpClientKind::Kiro => {
            let mut config = McpConfig::load()?;
            config.mcp_servers.remove(name);
            config.save()
        }
        McpClientKind::Codex => {
            let (path, mut doc) = load_codex_doc()?;
            if let Some(table) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
                table.remove(name);
            }
            save_codex_doc(&path, &doc)
        }
        McpClientKind::ClaudeCli => {
            let (path, mut root) = load_claude_root()?;
            if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
                servers.remove(name);
            }
            save_claude_root(&path, &root)
        }
    }
}

pub fn set_mcp_server_disabled_for_client(
    client: &str,
    server_name: &str,
    disabled: bool,
) -> Result<(), String> {
    with_mcp_config_write_lock(|| match McpClientKind::parse(client)? {
        McpClientKind::Kiro => {
            let mut config = McpConfig::load()?;
            let server = config
                .mcp_servers
                .get_mut(server_name)
                .ok_or_else(|| format!("kiro 中不存在 MCP 服务器 {server_name}"))?;
            match server {
                McpServer::Command(server) => server.disabled = disabled,
                McpServer::Url(server) => server.disabled = disabled,
            }
            config.save()
        }
        McpClientKind::Codex => {
            let (path, mut doc) = load_codex_doc()?;
            let server = codex_server_table_mut(&mut doc, server_name)?;
            server.insert("disabled", value(disabled));
            save_codex_doc(&path, &doc)
        }
        McpClientKind::ClaudeCli => {
            let (path, mut root) = load_claude_root()?;
            let server = claude_server_object_mut(&mut root, server_name)?;
            server.insert("disabled".to_string(), serde_json::Value::Bool(disabled));
            save_claude_root(&path, &root)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn codex_field_updates_preserve_unknown_nested_values() {
        let mut doc = r#"
[mcp_servers.demo]
url = "https://example.com/mcp"
disabled = false
timeout = 12.5
headers = { Authorization = "Bearer redacted", retries = [1, 2, 3] }

[mcp_servers.demo.metadata]
owner = "team-a"
"#
        .parse::<DocumentMut>()
        .unwrap();
        codex_server_table_mut(&mut doc, "demo")
            .unwrap()
            .insert("disabled", value(true));

        let server = codex_server_table_mut(&mut doc, "demo").unwrap();
        assert_eq!(server.get("disabled").and_then(Item::as_bool), Some(true));
        assert_eq!(server.get("timeout").and_then(Item::as_float), Some(12.5));
        assert_eq!(
            server
                .get("metadata")
                .and_then(Item::as_table)
                .and_then(|table| table.get("owner"))
                .and_then(Item::as_str),
            Some("team-a")
        );
        let raw = toml_value_to_json(&Item::Table(server.clone()));
        assert_eq!(raw["headers"]["retries"], serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn claude_field_updates_preserve_unknown_root_and_server_values() {
        let mut root = serde_json::json!({
            "theme": "dark",
            "mcpServers": {
                "demo": {
                    "url": "https://example.com/mcp",
                    "disabled": false,
                    "headers": { "X-Custom": "value" },
                    "timeout": 45
                }
            }
        });
        claude_server_object_mut(&mut root, "demo")
            .unwrap()
            .insert("disabled".to_string(), serde_json::Value::Bool(true));
        assert_eq!(root["theme"], "dark");
        assert_eq!(root["mcpServers"]["demo"]["headers"]["X-Custom"], "value");
        assert_eq!(root["mcpServers"]["demo"]["timeout"], 45);
        assert_eq!(root["mcpServers"]["demo"]["disabled"], true);
    }

    #[test]
    fn config_write_coordinator_serializes_concurrent_mutations() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = std::thread::spawn(move || {
            with_mcp_config_write_lock(|| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
            .unwrap();
        });
        entered_rx.recv().unwrap();

        let (second_tx, second_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            with_mcp_config_write_lock(|| {
                second_tx.send(()).unwrap();
                Ok(())
            })
            .unwrap();
        });
        assert!(second_rx.recv_timeout(Duration::from_millis(50)).is_err());
        release_tx.send(()).unwrap();
        second_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        first.join().unwrap();
        second.join().unwrap();
    }
}

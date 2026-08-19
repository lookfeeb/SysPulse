use super::external_sessions;
use crate::ai::backend::models::ide_session::{
    ContentItem, HistoryItem, IdeSession, Message, SessionPage, SessionSummary, SessionTree,
};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

// 安全限制
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB

/// CLI 会话工作区标识前缀：`cli:<cwd>`（按工作目录分组）
const CLI_PREFIX: &str = "cli:";

/// kiro-cli 会话元数据（~/.kiro/sessions/cli/<id>.json）
#[derive(Default, serde::Deserialize)]
struct CliSessionMeta {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    session_created_reason: Option<String>,
}

pub struct SessionStorage {
    base_path: PathBuf,
}

impl SessionStorage {
    pub fn new() -> Result<Self> {
        let base_path = Self::get_storage_path()?;
        Ok(Self { base_path })
    }

    /// 验证路径组件是否安全（防止路径遍历）
    fn is_safe_path_component(component: &str) -> bool {
        // 只允许字母、数字、下划线、连字符和点号
        // 不允许路径分隔符和特殊字符
        !component.is_empty()
            && !component.contains("..")
            && !component.contains('/')
            && !component.contains('\\')
            && component
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    }

    fn get_storage_path() -> Result<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            let appdata =
                std::env::var("APPDATA").context("Failed to get APPDATA environment variable")?;
            Ok(PathBuf::from(appdata)
                .join("Kiro")
                .join("User")
                .join("globalStorage")
                .join("kiro.kiroagent")
                .join("workspace-sessions"))
        }

        #[cfg(target_os = "macos")]
        {
            let home = std::env::var("HOME").context("Failed to get HOME environment variable")?;
            Ok(PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Kiro")
                .join("User")
                .join("globalStorage")
                .join("kiro.kiroagent")
                .join("workspace-sessions"))
        }

        #[cfg(target_os = "linux")]
        {
            let home = std::env::var("HOME").context("Failed to get HOME environment variable")?;
            Ok(PathBuf::from(home)
                .join(".config")
                .join("Kiro")
                .join("User")
                .join("globalStorage")
                .join("kiro.kiroagent")
                .join("workspace-sessions"))
        }
    }

    /// 列出所有 workspace
    pub fn list_workspaces(&self) -> Result<Vec<String>> {
        let mut workspaces = Vec::new();

        if self.base_path.exists() {
            // 收集工作区及其修改时间
            let mut workspace_with_time: Vec<(String, std::time::SystemTime)> = Vec::new();

            for entry in fs::read_dir(&self.base_path)
                .context("Failed to read workspace-sessions directory")?
            {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let modified = entry.metadata()?.modified()?;
                    workspace_with_time.push((name, modified));
                }
            }

            // 按修改时间倒序排序（最近使用的在前）
            workspace_with_time.sort_by_key(|item| std::cmp::Reverse(item.1));

            // 只返回名称
            workspaces = workspace_with_time
                .into_iter()
                .map(|(name, _)| name)
                .collect();
        }

        // CLI 会话按工作目录(cwd)分组为多个工作区，置顶
        let mut cli_ws: Vec<String> = Vec::new();
        for s in self.collect_cli_sessions(None) {
            let id = format!("{CLI_PREFIX}{}", s.workspace_directory);
            if !cli_ws.contains(&id) {
                cli_ws.push(id);
            }
        }
        cli_ws.extend(workspaces);
        workspaces = cli_ws;

        workspaces.extend(external_sessions::list_workspaces());

        Ok(workspaces)
    }

    /// 一次性列出 workspace 和会话摘要，供前端启动/刷新批量加载
    pub fn list_session_tree(&self) -> Result<SessionTree> {
        let mut workspaces = Vec::new();
        let mut sessions_by_workspace: HashMap<String, Vec<SessionSummary>> = HashMap::new();

        let cli_sessions = self.collect_cli_sessions(None);
        for session in cli_sessions {
            if !workspaces.contains(&session.workspace_hash) {
                workspaces.push(session.workspace_hash.clone());
            }
            sessions_by_workspace
                .entry(session.workspace_hash.clone())
                .or_default()
                .push(session);
        }

        if self.base_path.exists() {
            let mut workspace_with_time: Vec<(String, std::time::SystemTime)> = Vec::new();
            for entry in fs::read_dir(&self.base_path)
                .context("Failed to read workspace-sessions directory")?
            {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let modified = entry.metadata()?.modified()?;
                    workspace_with_time.push((name, modified));
                }
            }
            workspace_with_time.sort_by_key(|item| std::cmp::Reverse(item.1));

            for (workspace, _) in workspace_with_time {
                if !workspaces.contains(&workspace) {
                    workspaces.push(workspace.clone());
                }
                sessions_by_workspace.insert(workspace.clone(), self.list_sessions(&workspace)?);
            }
        }

        for session in external_sessions::list_all_sessions() {
            if !workspaces.contains(&session.workspace_hash) {
                workspaces.push(session.workspace_hash.clone());
            }
            sessions_by_workspace
                .entry(session.workspace_hash.clone())
                .or_default()
                .push(session);
        }

        for sessions in sessions_by_workspace.values_mut() {
            sessions.sort_by_key(|item| std::cmp::Reverse(item.modified_at.unwrap_or(0)));
        }

        Ok(SessionTree {
            workspaces,
            sessions_by_workspace,
        })
    }

    /// 列出指定 workspace 的所有 sessions
    pub fn list_sessions(&self, workspace_hash: &str) -> Result<Vec<SessionSummary>> {
        if external_sessions::handles(workspace_hash) {
            return Ok(external_sessions::list_sessions(workspace_hash));
        }
        if let Some(cwd) = workspace_hash.strip_prefix(CLI_PREFIX) {
            return Ok(self.collect_cli_sessions(Some(cwd)));
        }
        // 安全检查：防止路径遍历攻击
        if !Self::is_safe_path_component(workspace_hash) {
            log::warn!("[安全] 检测到非法的 workspace_hash: {}", workspace_hash);
            return Err(anyhow::anyhow!("Invalid workspace hash"));
        }

        let workspace_path = self.base_path.join(workspace_hash);
        let mut sessions = Vec::new();

        if !workspace_path.exists() {
            log::warn!("Workspace directory does not exist: {}", workspace_hash);
            return Ok(sessions);
        }

        for entry in fs::read_dir(&workspace_path).context(format!(
            "Failed to read workspace directory: {}",
            workspace_hash
        ))? {
            let entry = entry?;
            let path = entry.path();

            // 只处理 .json 文件
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            // 跳过 sessions.json（索引文件）
            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if filename == "sessions.json" {
                    continue;
                }
            }

            match self.load_session_summary(&path, workspace_hash) {
                Ok(summary) => sessions.push(summary),
                Err(e) => {
                    log::error!("Failed to load session from {:?}: {}", path, e);
                    // 继续处理其他文件
                }
            }
        }

        // 按修改时间倒序排序
        sessions.sort_by_key(|item| std::cmp::Reverse(item.modified_at.unwrap_or(0)));

        Ok(sessions)
    }

    /// 加载 session 摘要
    fn load_session_summary(&self, path: &PathBuf, workspace_hash: &str) -> Result<SessionSummary> {
        let metadata =
            fs::metadata(path).context(format!("Failed to read metadata for {:?}", path))?;

        // 安全检查：文件大小限制
        if metadata.len() > MAX_FILE_SIZE {
            return Err(anyhow::anyhow!("File too large: {} bytes", metadata.len()));
        }

        let content =
            fs::read_to_string(path).context(format!("Failed to read file {:?}", path))?;

        let session: IdeSession = serde_json::from_str(&content)
            .map_err(|e| {
                log::error!("JSON parse error for {:?}: {}", path, e);
                e
            })
            .context(format!("Failed to parse JSON from {:?}", path))?;

        Ok(SessionSummary {
            session_id: session.session_id,
            title: session.title,
            session_type: session.session_type,
            workspace_directory: session.workspace_directory,
            workspace_hash: workspace_hash.to_string(),
            message_count: session.history.len(),
            file_size: metadata.len(),
            created_at: metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
            modified_at: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64),
            source: "ide".to_string(),
        })
    }

    /// 返回 session 文件在磁盘上的真实完整路径
    pub fn get_session_file_path(&self, workspace_hash: &str, session_id: &str) -> Result<String> {
        if external_sessions::handles(workspace_hash) {
            return external_sessions::file_path(workspace_hash, session_id);
        }
        if !Self::is_safe_path_component(session_id) {
            return Err(anyhow::anyhow!("Invalid session id"));
        }
        // CLI 会话：~/.kiro/sessions/cli/<id>.jsonl
        if workspace_hash.starts_with(CLI_PREFIX) {
            let dir = Self::cli_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
            return Ok(dir
                .join(format!("{session_id}.jsonl"))
                .to_string_lossy()
                .to_string());
        }
        // IDE 会话：<base>/<workspace_hash>/<id>.json
        if !Self::is_safe_path_component(workspace_hash) {
            return Err(anyhow::anyhow!("Invalid workspace hash"));
        }
        Ok(self
            .base_path
            .join(workspace_hash)
            .join(format!("{session_id}.json"))
            .to_string_lossy()
            .to_string())
    }

    /// 加载完整 session
    pub fn load_session(&self, workspace_hash: &str, session_id: &str) -> Result<IdeSession> {
        if external_sessions::handles(workspace_hash) {
            return external_sessions::load_session(workspace_hash, session_id);
        }
        if workspace_hash.starts_with(CLI_PREFIX) {
            return self.load_cli_session(session_id);
        }
        // 安全检查：防止路径遍历攻击
        if !Self::is_safe_path_component(workspace_hash)
            || !Self::is_safe_path_component(session_id)
        {
            log::warn!(
                "[安全] 检测到非法的路径参数: workspace_hash={}, session_id={}",
                workspace_hash,
                session_id
            );
            return Err(anyhow::anyhow!("Invalid path parameters"));
        }

        let path = self
            .base_path
            .join(workspace_hash)
            .join(format!("{}.json", session_id));

        // 安全检查：文件大小限制
        let metadata = fs::metadata(&path).context(format!(
            "Failed to read metadata for session: {}",
            session_id
        ))?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(anyhow::anyhow!(
                "Session file too large: {} bytes",
                metadata.len()
            ));
        }

        let content = fs::read_to_string(&path)
            .context(format!("Failed to read session file: {}", session_id))?;
        let session = serde_json::from_str(&content).context("Failed to parse session JSON")?;
        Ok(session)
    }

    pub fn load_session_page(
        &self,
        workspace_hash: &str,
        session_id: &str,
        page: usize,
        page_size: usize,
    ) -> Result<SessionPage> {
        if external_sessions::handles(workspace_hash) {
            return external_sessions::load_session_page(
                workspace_hash,
                session_id,
                page,
                page_size,
            );
        }
        if workspace_hash.starts_with(CLI_PREFIX) {
            return self.load_cli_session_page(session_id, page, page_size);
        }
        Ok(SessionPage::from_session(
            self.load_session(workspace_hash, session_id)?,
            page,
            page_size,
        ))
    }

    /// 删除 session
    pub fn delete_session(&self, workspace_hash: &str, session_id: &str) -> Result<()> {
        if external_sessions::handles(workspace_hash) {
            return external_sessions::delete_session(workspace_hash, session_id);
        }
        let _guard = external_sessions::lock_session_mutations()?;
        if workspace_hash.starts_with(CLI_PREFIX) {
            return self.delete_cli_session(session_id);
        }
        // 安全检查：防止路径遍历攻击
        if !Self::is_safe_path_component(workspace_hash)
            || !Self::is_safe_path_component(session_id)
        {
            log::warn!(
                "[安全] 检测到非法的路径参数: workspace_hash={}, session_id={}",
                workspace_hash,
                session_id
            );
            return Err(anyhow::anyhow!("Invalid path parameters"));
        }

        let path = self
            .base_path
            .join(workspace_hash)
            .join(format!("{}.json", session_id));

        fs::remove_file(&path).context(format!("Failed to delete session: {}", session_id))?;

        Ok(())
    }

    /// 删除整个工作区目录
    pub fn delete_workspace(&self, workspace_hash: &str) -> Result<()> {
        if external_sessions::handles(workspace_hash) {
            return external_sessions::delete_workspace(workspace_hash);
        }
        let _guard = external_sessions::lock_session_mutations()?;
        if let Some(cwd) = workspace_hash.strip_prefix(CLI_PREFIX) {
            return self.delete_cli_workspace(cwd);
        }
        // 安全检查：防止路径遍历攻击
        if !Self::is_safe_path_component(workspace_hash) {
            log::warn!("[安全] 检测到非法的 workspace_hash: {}", workspace_hash);
            return Err(anyhow::anyhow!("Invalid workspace hash"));
        }

        let workspace_path = self.base_path.join(workspace_hash);

        if workspace_path.exists() {
            fs::remove_dir_all(&workspace_path)
                .context(format!("Failed to delete workspace: {}", workspace_hash))?;
        }

        Ok(())
    }

    /// 导出 session
    pub fn export_session_to_path(
        &self,
        workspace_hash: &str,
        session_id: &str,
        format: ExportFormat,
        path: &Path,
    ) -> Result<()> {
        let session = self.load_session(workspace_hash, session_id)?;
        crate::ai::backend::utils::fs::atomic_write_with(path, "AI 会话导出", |file| {
            let mut writer = BufWriter::new(file);
            match format {
                ExportFormat::Json => {
                    serde_json::to_writer_pretty(&mut writer, &session)
                        .map_err(|error| format!("序列化会话 JSON 失败: {error}"))?;
                    writer
                        .write_all(b"\n")
                        .map_err(|error| format!("写入会话 JSON 失败: {error}"))?;
                }
                ExportFormat::Markdown => self.write_session_markdown(&mut writer, &session)?,
            }
            writer
                .flush()
                .map_err(|error| format!("刷新会话导出缓冲区失败: {error}"))
        })
        .map_err(anyhow::Error::msg)
    }

    fn write_session_markdown(
        &self,
        writer: &mut impl Write,
        session: &IdeSession,
    ) -> Result<(), String> {
        writeln!(writer, "# {}\n", session.title)
            .map_err(|error| format!("写入会话标题失败: {error}"))?;
        writeln!(writer, "- **Session ID**: {}", session.session_id)
            .map_err(|error| format!("写入会话信息失败: {error}"))?;
        writeln!(writer, "- **Type**: {}", session.session_type)
            .map_err(|error| format!("写入会话信息失败: {error}"))?;
        writeln!(writer, "- **Workspace**: {}", session.workspace_directory)
            .map_err(|error| format!("写入会话信息失败: {error}"))?;
        writeln!(writer, "- **Messages**: {}\n\n---\n", session.history.len())
            .map_err(|error| format!("写入会话信息失败: {error}"))?;
        for (i, item) in session.history.iter().enumerate() {
            writeln!(
                writer,
                "## Message {}\n\n**{}**:\n",
                i + 1,
                if item.message.role == "user" {
                    "User"
                } else {
                    "Assistant"
                }
            )
            .map_err(|error| format!("写入会话消息失败: {error}"))?;
            for content in &item.message.content {
                writeln!(writer, "{}\n", content.text)
                    .map_err(|error| format!("写入会话消息失败: {error}"))?;
            }
            writer
                .write_all(b"---\n\n")
                .map_err(|error| format!("写入会话分隔符失败: {error}"))?;
        }
        Ok(())
    }

    // ===== Kiro CLI 会话来源（~/.kiro/sessions/cli/）=====

    fn cli_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".kiro").join("sessions").join("cli"))
    }

    fn parse_iso_secs(s: &Option<String>) -> Option<i64> {
        s.as_ref()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|dt| dt.timestamp())
    }

    /// 统计 jsonl 中的消息数（Prompt + AssistantMessage）与文件大小
    fn cli_jsonl_stats(jsonl: &Path) -> (usize, u64) {
        let size = fs::metadata(jsonl).map(|m| m.len()).unwrap_or(0);
        if size == 0 || size > MAX_FILE_SIZE {
            return (0, size);
        }
        let count = fs::File::open(jsonl)
            .map(|file| {
                BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .filter(|l| {
                        serde_json::from_str::<serde_json::Value>(l)
                            .ok()
                            .and_then(|v| {
                                v.get("kind")
                                    .and_then(|k| k.as_str())
                                    .map(|k| k == "Prompt" || k == "AssistantMessage")
                            })
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        (count, size)
    }

    /// 收集 CLI 会话（仅含真实对话的）；filter_cwd 为 Some 时只返回该工作目录下的会话
    fn collect_cli_sessions(&self, filter_cwd: Option<&str>) -> Vec<SessionSummary> {
        let dir = match Self::cli_dir() {
            Some(d) if d.is_dir() => d,
            _ => return Vec::new(),
        };
        let read = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut sessions = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                eprintln!(
                    "[SessionStorage] 读取 CLI 会话元数据失败: {}",
                    path.display()
                );
                continue;
            };
            let Ok(meta) = serde_json::from_str::<CliSessionMeta>(&content) else {
                eprintln!(
                    "[SessionStorage] 解析 CLI 会话元数据失败: {}",
                    path.display()
                );
                continue;
            };
            if meta.session_id.is_empty() {
                continue;
            }
            let cwd = meta.cwd.clone().unwrap_or_default();
            if let Some(f) = filter_cwd {
                if cwd != f {
                    continue;
                }
            }
            let (msg_count, file_size) = Self::cli_jsonl_stats(&path.with_extension("jsonl"));
            // 仅展示有真实对话（含 Prompt/AssistantMessage）的会话，跳过纯元数据会话
            if msg_count == 0 {
                continue;
            }
            let file_mtime = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let title = meta
                .title
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| meta.session_id.clone());
            sessions.push(SessionSummary {
                title,
                session_type: meta
                    .session_created_reason
                    .unwrap_or_else(|| "cli".to_string()),
                workspace_hash: format!("{CLI_PREFIX}{cwd}"),
                workspace_directory: cwd,
                message_count: msg_count,
                file_size,
                created_at: Self::parse_iso_secs(&meta.created_at),
                modified_at: Self::parse_iso_secs(&meta.updated_at).or(file_mtime),
                source: "cli".to_string(),
                session_id: meta.session_id,
            });
        }
        sessions.sort_by_key(|item| std::cmp::Reverse(item.modified_at.unwrap_or(0)));
        sessions
    }

    fn load_cli_session(&self, session_id: &str) -> Result<IdeSession> {
        let (dir, meta) = Self::load_cli_session_meta(session_id)?;

        let jsonl_path = dir.join(format!("{session_id}.jsonl"));
        let mut history = Vec::new();
        if jsonl_path.exists() {
            let size = fs::metadata(&jsonl_path).map(|m| m.len()).unwrap_or(0);
            if size > MAX_FILE_SIZE {
                return Err(anyhow::anyhow!("Session file too large: {} bytes", size));
            }
            let file = fs::File::open(&jsonl_path)
                .context("Failed to read kiro-cli session transcript")?;
            for (i, line) in BufReader::new(file).lines().enumerate() {
                let line = line.context("Failed to read kiro-cli session transcript")?;
                if let Some(item) = Self::cli_line_to_history(&line, i) {
                    history.push(item);
                }
            }
        }

        Ok(IdeSession {
            title: meta
                .title
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| session_id.to_string()),
            session_type: meta
                .session_created_reason
                .unwrap_or_else(|| "cli".to_string()),
            workspace_directory: meta.cwd.unwrap_or_default(),
            history,
            conversation_summary: None,
            session_id: session_id.to_string(),
        })
    }

    fn load_cli_session_page(
        &self,
        session_id: &str,
        page: usize,
        page_size: usize,
    ) -> Result<SessionPage> {
        let (dir, meta) = Self::load_cli_session_meta(session_id)?;
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let start = page.saturating_sub(1).saturating_mul(page_size);
        let end = start.saturating_add(page_size);
        let jsonl_path = dir.join(format!("{session_id}.jsonl"));
        let (history, total_messages) = if jsonl_path.exists() {
            let size = fs::metadata(&jsonl_path).map(|m| m.len()).unwrap_or(0);
            if size > MAX_FILE_SIZE {
                return Err(anyhow::anyhow!("Session file too large: {} bytes", size));
            }
            let file = fs::File::open(&jsonl_path)
                .context("Failed to read kiro-cli session transcript")?;
            Self::read_cli_history_page(BufReader::new(file), start, end, page_size)?
        } else {
            (Vec::new(), 0)
        };

        Ok(SessionPage {
            session_id: session_id.to_string(),
            title: meta
                .title
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| session_id.to_string()),
            session_type: meta
                .session_created_reason
                .unwrap_or_else(|| "cli".to_string()),
            workspace_directory: meta.cwd.unwrap_or_default(),
            history,
            conversation_summary: None,
            total_messages,
            page,
            page_size,
        })
    }

    fn load_cli_session_meta(session_id: &str) -> Result<(PathBuf, CliSessionMeta)> {
        if !Self::is_safe_path_component(session_id) {
            return Err(anyhow::anyhow!("Invalid session id"));
        }
        let dir = Self::cli_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
        let meta_path = dir.join(format!("{session_id}.json"));
        let meta_content = fs::read_to_string(&meta_path).with_context(|| {
            format!(
                "Failed to read kiro-cli session metadata: {}",
                meta_path.display()
            )
        })?;
        let meta: CliSessionMeta = serde_json::from_str(&meta_content).with_context(|| {
            format!(
                "Failed to parse kiro-cli session metadata: {}",
                meta_path.display()
            )
        })?;
        Ok((dir, meta))
    }

    fn read_cli_history_page(
        reader: impl BufRead,
        start: usize,
        end: usize,
        page_size: usize,
    ) -> Result<(Vec<HistoryItem>, usize)> {
        let mut history = Vec::with_capacity(page_size);
        let mut total_messages = 0usize;
        for (line_index, line) in reader.lines().enumerate() {
            let line = line.context("Failed to read kiro-cli session transcript")?;
            let Some(item) = Self::cli_line_to_history(&line, line_index) else {
                continue;
            };
            let message_index = total_messages;
            total_messages = total_messages.saturating_add(1);
            if message_index >= start && message_index < end {
                history.push(item);
            }
        }
        Ok((history, total_messages))
    }

    /// 把一行 jsonl 转成 HistoryItem；非对话行（ToolResults 等）返回 None
    fn cli_line_to_history(line: &str, idx: usize) -> Option<HistoryItem> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let role = match v.get("kind")?.as_str()? {
            "Prompt" => "user",
            "AssistantMessage" => "assistant",
            _ => return None,
        };
        let data = v.get("data")?;
        let id = data
            .get("message_id")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("cli-{idx}"));
        let mut items = Vec::new();
        if let Some(arr) = data.get("content").and_then(|c| c.as_array()) {
            for c in arr {
                let text = match c.get("kind").and_then(|x| x.as_str()).unwrap_or("") {
                    "text" => c
                        .get("data")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    "toolUse" => format!(
                        "🔧 调用工具: {}",
                        c.get("data")
                            .and_then(|d| d.get("name"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("?")
                    ),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    items.push(ContentItem {
                        content_type: "text".to_string(),
                        text,
                    });
                }
            }
        }
        if items.is_empty() {
            return None;
        }
        Some(HistoryItem {
            message: Message {
                role: role.to_string(),
                content: items,
                is_hidden: false,
                id,
            },
            context_items: Vec::new(),
            editor_state: serde_json::Value::Null,
            prompt_logs: Vec::new(),
        })
    }

    fn delete_cli_session(&self, session_id: &str) -> Result<()> {
        if !Self::is_safe_path_component(session_id) {
            return Err(anyhow::anyhow!("Invalid session id"));
        }
        let dir = Self::cli_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
        let mut errors = Vec::new();
        for ext in ["json", "jsonl", "history", "lock"] {
            let p = dir.join(format!("{session_id}.{ext}"));
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => errors.push(format!("删除 {} 失败: {error}", p.display())),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(errors.join("；")))
        }
    }

    /// 删除某工作目录(cwd)下的所有 CLI 会话
    fn delete_cli_workspace(&self, cwd: &str) -> Result<()> {
        let dir = match Self::cli_dir() {
            Some(d) if d.is_dir() => d,
            _ => return Ok(()),
        };
        let mut errors = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(format!("读取 Kiro CLI 会话目录项失败: {error}"));
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                eprintln!(
                    "[SessionStorage] 读取 CLI 会话元数据失败: {}",
                    path.display()
                );
                continue;
            };
            let Ok(meta) = serde_json::from_str::<CliSessionMeta>(&content) else {
                eprintln!(
                    "[SessionStorage] 解析 CLI 会话元数据失败: {}",
                    path.display()
                );
                continue;
            };
            if meta.cwd.as_deref().unwrap_or_default() == cwd && !meta.session_id.is_empty() {
                if let Err(error) = self.delete_cli_session(&meta.session_id) {
                    errors.push(format!("{}: {error}", meta.session_id));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(errors.join("；")))
        }
    }
}

pub enum ExportFormat {
    Json,
    Markdown,
}

#[cfg(test)]
mod session_storage_tests {
    use super::*;

    fn cli_line(kind: &str, id: &str, text: &str) -> String {
        serde_json::json!({
            "kind": kind,
            "data": {
                "message_id": id,
                "content": [{ "kind": "text", "data": text }]
            }
        })
        .to_string()
    }

    #[test]
    fn kiro_cli_page_counts_only_conversation_lines_and_keeps_boundaries() {
        let content = [
            cli_line("Prompt", "m1", "第一条"),
            serde_json::json!({ "kind": "ToolResults", "data": {} }).to_string(),
            cli_line("AssistantMessage", "m2", "第二条"),
            cli_line("Prompt", "m3", "第三条"),
            cli_line("AssistantMessage", "m4", "第四条"),
            cli_line("Prompt", "m5", "第五条"),
        ]
        .join("\n");
        let (history, total) =
            SessionStorage::read_cli_history_page(BufReader::new(content.as_bytes()), 2, 4, 2)
                .unwrap();
        assert_eq!(total, 5);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].message.id, "m3");
        assert_eq!(history[1].message.id, "m4");
    }

    #[test]
    fn export_streams_to_atomic_json_and_markdown_files() {
        let dir =
            std::env::temp_dir().join(format!("syspulse-session-export-{}", uuid::Uuid::new_v4()));
        let base = dir.join("sessions");
        let workspace = base.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("session.json"),
            serde_json::to_vec(&serde_json::json!({
                "sessionId": "session",
                "title": "导出测试",
                "sessionType": "ide",
                "workspaceDirectory": "C:\\Workspace",
                "history": [{
                    "message": {
                        "role": "user",
                        "content": "第一条消息",
                        "isHidden": false,
                        "id": "message-1"
                    },
                    "contextItems": [],
                    "editorState": null,
                    "promptLogs": []
                }],
                "conversationSummary": "摘要"
            }))
            .unwrap(),
        )
        .unwrap();
        let storage = SessionStorage { base_path: base };
        let json_path = dir.join("session-export.json");
        let markdown_path = dir.join("session-export.md");

        storage
            .export_session_to_path("workspace", "session", ExportFormat::Json, &json_path)
            .unwrap();
        storage
            .export_session_to_path(
                "workspace",
                "session",
                ExportFormat::Markdown,
                &markdown_path,
            )
            .unwrap();

        let exported_json: serde_json::Value =
            serde_json::from_slice(&fs::read(&json_path).unwrap()).unwrap();
        assert_eq!(exported_json["title"], "导出测试");
        let markdown = fs::read_to_string(&markdown_path).unwrap();
        assert!(markdown.contains("# 导出测试"));
        assert!(markdown.contains("第一条消息"));
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(
                    |entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("tmp")
                )
                .count(),
            0
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

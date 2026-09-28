// 外部 AI 历史会话解析：Codex / Claude / Antigravity / Gemini CLI
// 统一映射到既有 SessionSummary / IdeSession 模型，作为新的 source 接入会话管理。
// 详见 session-history-parsing.md。

use std::collections::{HashMap, HashSet};
use std::fs;
#[cfg(test)]
use std::io::Write;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::ai::backend::models::ide_session::{
    ContentItem, HistoryItem, IdeSession, Message, SessionPage, SessionSummary,
};
use base64::{engine::general_purpose, Engine as _};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

#[path = "external_sessions/file_cache.rs"]
mod file_cache;
use file_cache::*;
#[path = "external_sessions/codex_pages.rs"]
mod codex_pages;
use codex_pages::load_codex_file_page;
#[path = "external_sessions/codex_metadata_cache.rs"]
mod codex_metadata_cache;
use codex_metadata_cache::*;

const MAX_TITLE_CHARS: usize = 72;
const CACHE_TTL: Duration = Duration::from_secs(5);

/// 进程内短期缓存：避免一次刷新中 list_workspaces + N×list_sessions 重复全量扫描
static CACHE: Mutex<Option<(Instant, Vec<SessionSummary>)>> = Mutex::new(None);
static SCAN_LOCK: Mutex<()> = Mutex::new(());
static CACHE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SESSION_MUTATION_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock_session_mutations() -> anyhow::Result<std::sync::MutexGuard<'static, ()>> {
    SESSION_MUTATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("AI 会话变更锁已损坏"))
}

/// 失效缓存（刷新按钮强制重扫）
pub fn invalidate_cache() {
    clear_codex_metadata_cache();
    if let Ok(mut cache) = CACHE.lock() {
        CACHE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *cache = None;
    }
}

/// 会话在磁盘上的组织方式
#[derive(Debug, Clone, Copy)]
enum Layout {
    /// 递归扫描 root 下的 *.jsonl，每文件一会话；session_id = 相对路径
    File { depth: usize },
    /// Antigravity: root/conversations/*.pb，摘要在 root/agyhub_summaries_proto.pb
    Antigravity,
    /// Antigravity IDE: root/brain/<uuid> 聚合 task / plan / walkthrough / transcript
    AntigravityIde,
    /// Gemini CLI: ~/.gemini/tmp/<project>/chats/*.jsonl
    Gemini,
}

/// 一个外部 CLI 历史来源的定义。
/// 新增其它 CLI：只需在 SOURCES 增加一项 + 写解析函数即可，无需改动分发逻辑。
struct SourceDef {
    prefix: &'static str,
    source: &'static str,
    root: fn() -> Option<PathBuf>,
    layout: Layout,
    parse: fn(&str) -> Parsed,
    scan: fn(&Path) -> Option<ParsedSummary>,
}

static SOURCES: &[SourceDef] = &[
    SourceDef {
        prefix: "codex:",
        source: "codex",
        root: codex_root,
        layout: Layout::File { depth: 8 },
        parse: parse_codex,
        scan: scan_codex_summary,
    },
    SourceDef {
        prefix: "claude:",
        source: "claude",
        root: claude_root,
        layout: Layout::File { depth: 4 },
        parse: parse_claude,
        scan: scan_claude_summary,
    },
    SourceDef {
        prefix: "antigravity:",
        source: "antigravity",
        root: antigravity_root,
        layout: Layout::Antigravity,
        parse: parse_antigravity,
        scan: |_| None,
    },
    SourceDef {
        prefix: "antigravity-backup:",
        source: "antigravity-backup",
        root: antigravity_backup_root,
        layout: Layout::Antigravity,
        parse: parse_antigravity,
        scan: |_| None,
    },
    SourceDef {
        prefix: "antigravity-ide:",
        source: "antigravity-ide",
        root: antigravity_ide_root,
        layout: Layout::AntigravityIde,
        parse: parse_antigravity,
        scan: |_| None,
    },
    SourceDef {
        prefix: "gemini:",
        source: "gemini",
        root: gemini_tmp_root,
        layout: Layout::Gemini,
        parse: parse_gemini,
        scan: scan_gemini_summary,
    },
];

fn def_for(hash: &str) -> Option<&'static SourceDef> {
    SOURCES.iter().find(|d| hash.starts_with(d.prefix))
}

/// 本模块是否负责该 workspace_hash
pub fn handles(hash: &str) -> bool {
    def_for(hash).is_some()
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".codex")))
}
fn codex_root() -> Option<PathBuf> {
    codex_home().map(|h| h.join("sessions"))
}
fn claude_home() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".claude")))
}
fn claude_root() -> Option<PathBuf> {
    claude_home().map(|h| h.join("projects"))
}
fn antigravity_root() -> Option<PathBuf> {
    home().map(|h| h.join(".gemini").join("antigravity"))
}
fn antigravity_backup_root() -> Option<PathBuf> {
    home().map(|h| h.join(".gemini").join("antigravity-backup"))
}
fn antigravity_ide_root() -> Option<PathBuf> {
    home().map(|h| h.join(".gemini").join("antigravity-ide"))
}
fn gemini_tmp_root() -> Option<PathBuf> {
    home().map(|h| h.join(".gemini").join("tmp"))
}

/// 解析后的中间结构
struct Parsed {
    cwd: String,
    title: String,
    created: Option<i64>,
    updated: Option<i64>,
    blocks: Vec<(String, String)>, // (role, text)
}

struct PagedBlocks {
    page: usize,
    page_size: usize,
    start: usize,
    end: usize,
    raw_count: usize,
    retained: Vec<(usize, String, String)>,
    title: Option<String>,
}

impl PagedBlocks {
    fn new(page: usize, page_size: usize) -> Self {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let start = page.saturating_sub(1).saturating_mul(page_size);
        let end = start.saturating_add(page_size);
        Self {
            page,
            page_size,
            start,
            end,
            raw_count: 0,
            retained: Vec::with_capacity(page_size.saturating_add(1)),
            title: None,
        }
    }

    fn push(&mut self, role: &str, text: String) {
        if text.trim().is_empty() {
            return;
        }
        if role == "user" && self.title.is_none() {
            self.title = meaningful_line(&text).map(|line| truncate(&line, MAX_TITLE_CHARS));
        }
        let raw_index = self.raw_count;
        self.raw_count = self.raw_count.saturating_add(1);
        if raw_index >= self.start.saturating_sub(1) && raw_index < self.end {
            self.retained.push((raw_index, role.to_string(), text));
        }
    }

    fn finish(
        self,
        session_id: &str,
        session_type: &str,
        workspace_directory: String,
        title_override: Option<String>,
    ) -> SessionPage {
        let cwd_prefix = usize::from(!workspace_directory.is_empty());
        let total_messages = self.raw_count.saturating_add(cwd_prefix);
        let mut history = Vec::with_capacity(self.page_size.min(total_messages));
        if cwd_prefix == 1 && self.start == 0 {
            history.push(history_item(
                "system",
                format!("工作目录：{workspace_directory}"),
                0,
            ));
        }
        for (raw_index, role, text) in self.retained {
            let final_index = raw_index.saturating_add(cwd_prefix);
            if final_index >= self.start && final_index < self.end {
                history.push(history_item(&role, text, raw_index.saturating_add(1)));
            }
        }
        let title = title_override
            .filter(|value| !value.trim().is_empty())
            .or(self.title)
            .unwrap_or_else(|| "未命名会话".to_string());
        SessionPage {
            session_id: session_id.to_string(),
            title,
            session_type: session_type.to_string(),
            workspace_directory,
            history,
            conversation_summary: None,
            total_messages,
            page: self.page,
            page_size: self.page_size,
        }
    }
}

#[derive(Clone)]
struct ParsedSummary {
    stable_id: Option<String>,
    cwd: String,
    title: String,
    created: Option<i64>,
    updated: Option<i64>,
    message_count: usize,
}

#[derive(Default, serde::Deserialize)]
struct AntigravityArtifactMeta {
    #[serde(default)]
    summary: String,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
}

#[derive(Clone, Default)]
struct AntigravityIdeIndexEntry {
    title: String,
    cwd: String,
}

include!("external_sessions/common.rs");
#[path = "external_sessions/antigravity/protobuf.rs"]
mod antigravity_protobuf;
use antigravity_protobuf::*;
#[path = "external_sessions/antigravity/metadata.rs"]
mod antigravity_metadata;
use antigravity_metadata::*;
#[path = "external_sessions/antigravity/catalog.rs"]
mod antigravity_catalog;
use antigravity_catalog::*;
#[path = "external_sessions/antigravity/loading.rs"]
mod antigravity_loading;
use antigravity_loading::*;
#[path = "external_sessions/antigravity/deletion.rs"]
mod antigravity_deletion;
use antigravity_deletion::*;
#[path = "external_sessions/antigravity/parsing.rs"]
mod antigravity_parsing;
use antigravity_parsing::*;
#[cfg(test)]
#[path = "external_sessions/antigravity/tests.rs"]
mod antigravity_tests;
#[path = "external_sessions/codex/parsing.rs"]
mod codex_parsing;
use codex_parsing::*;
#[path = "external_sessions/codex/metadata.rs"]
mod codex_metadata;
use codex_metadata::*;
#[path = "external_sessions/codex/catalog.rs"]
mod codex_catalog;
use codex_catalog::*;
#[path = "external_sessions/codex/loading.rs"]
mod codex_loading;
use codex_loading::*;
#[path = "external_sessions/codex/deletion.rs"]
mod codex_deletion;
use codex_deletion::*;
#[path = "external_sessions/claude/parsing.rs"]
mod claude_parsing;
#[cfg(test)]
#[path = "external_sessions/codex/tests.rs"]
mod codex_delete_tests;
use claude_parsing::*;
#[path = "external_sessions/claude/loading.rs"]
mod claude_loading;
use claude_loading::*;
#[path = "external_sessions/claude/summary.rs"]
mod claude_summary;
use claude_summary::*;
#[path = "external_sessions/claude/metadata.rs"]
mod claude_metadata;
use claude_metadata::*;
#[path = "external_sessions/claude/catalog.rs"]
mod claude_catalog;
use claude_catalog::*;
#[path = "external_sessions/claude/deletion.rs"]
mod claude_deletion;
use claude_deletion::*;
#[cfg(test)]
#[path = "external_sessions/claude/tests.rs"]
mod claude_tests;
include!("external_sessions/gemini.rs");
include!("external_sessions/catalog.rs");
include!("external_sessions/operations.rs");

#[cfg(test)]
mod mutation_lock_tests {
    use super::lock_session_mutations;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn session_mutation_lock_serializes_concurrent_deletes() {
        let first_guard = lock_session_mutations().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let _guard = lock_session_mutations().unwrap();
            entered_tx.send(()).unwrap();
        });
        assert!(entered_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first_guard);
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        waiter.join().unwrap();
    }
}

use super::*;

// ===== Codex =====

#[derive(serde::Deserialize)]
pub(super) struct CodexEnvelope<'a> {
    #[serde(default, borrow)]
    pub(super) timestamp: Option<std::borrow::Cow<'a, str>>,
    #[serde(rename = "type", default, borrow)]
    pub(super) kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) payload: Option<CodexPayload<'a>>,
}

#[derive(serde::Deserialize)]
pub(super) struct CodexPayload<'a> {
    #[serde(rename = "type", default, borrow)]
    pub(super) kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) role: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) cwd: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) id: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) session_id: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) message: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) content: Vec<CodexContent<'a>>,
}

#[derive(serde::Deserialize)]
pub(super) struct CodexContent<'a> {
    #[serde(rename = "type", default, borrow)]
    pub(super) kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    pub(super) text: Option<std::borrow::Cow<'a, str>>,
}

pub(super) fn codex_summary_cache() -> &'static SummaryCache {
    static CACHE: OnceLock<SummaryCache> = OnceLock::new();
    CACHE.get_or_init(SummaryCache::default)
}

pub(super) fn prune_codex_summary_cache(live_paths: &HashSet<PathBuf>) {
    if let Ok(mut cache) = codex_summary_cache().lock() {
        cache.retain(|path, _| live_paths.contains(path));
    }
}

pub(super) fn parse_codex(content: &str) -> Parsed {
    parse_codex_reader(BufReader::new(content.as_bytes()))
}

pub(super) fn parse_codex_file(path: &Path) -> anyhow::Result<Parsed> {
    Ok(parse_codex_reader(BufReader::new(fs::File::open(path)?)))
}

pub(super) fn load_codex_file_page_uncached(
    d: &SourceDef,
    path: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
    cwd_fallback: Option<String>,
    title_override: Option<String>,
) -> anyhow::Result<SessionPage> {
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut blocks = PagedBlocks::new(page, page_size);
    let mut cwd = String::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let Ok(event) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        let ty = event.kind.as_deref().unwrap_or_default();
        let payload = event.payload.as_ref();
        let payload_type = payload
            .and_then(|value| value.kind.as_deref())
            .unwrap_or_default();
        match (ty, payload_type) {
            ("session_meta", _) => {
                if let Some(value) = payload.and_then(|value| value.cwd.as_deref()) {
                    cwd = value.to_string();
                }
            }
            ("event_msg", "user_message") => {
                if let Some(message) = payload.and_then(|value| value.message.as_deref()) {
                    blocks.push("user", message.to_string());
                }
            }
            ("response_item", "message")
                if payload.and_then(|value| value.role.as_deref()) == Some("assistant") =>
            {
                let mut text = String::new();
                if let Some(payload) = payload {
                    for item in &payload.content {
                        if matches!(item.kind.as_deref(), Some("output_text" | "input_text")) {
                            if let Some(value) = item.text.as_deref() {
                                text.push_str(value);
                            }
                        }
                    }
                }
                blocks.push("assistant", text);
            }
            _ => {}
        }
    }
    if cwd.is_empty() {
        cwd = cwd_fallback.unwrap_or_default();
    }
    Ok(blocks.finish(session_id, d.source, cwd, title_override))
}

pub(super) fn parse_codex_reader(mut reader: impl BufRead) -> Parsed {
    let mut p = Parsed {
        cwd: String::new(),
        title: String::new(),
        created: None,
        updated: None,
        blocks: Vec::new(),
    };
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            break;
        };
        if read == 0 {
            break;
        }
        let Ok(event) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        if let Some(ts) = event.timestamp.as_deref().and_then(iso_secs) {
            p.updated = Some(ts);
            if p.created.is_none() {
                p.created = Some(ts);
            }
        }
        let ty = event.kind.as_deref().unwrap_or_default();
        let payload = event.payload.as_ref();
        let pty = payload
            .and_then(|value| value.kind.as_deref())
            .unwrap_or_default();
        match (ty, pty) {
            ("session_meta", _) => {
                if let Some(cwd) = payload.and_then(|value| value.cwd.as_deref()) {
                    p.cwd = cwd.to_string();
                }
            }
            ("event_msg", "user_message") => {
                if let Some(m) = payload.and_then(|value| value.message.as_deref()) {
                    push_block(&mut p.blocks, "user", m.to_string());
                }
            }
            ("response_item", "message")
                if payload.and_then(|value| value.role.as_deref()) == Some("assistant") =>
            {
                let mut text = String::new();
                if let Some(payload) = payload {
                    for item in &payload.content {
                        if matches!(item.kind.as_deref(), Some("output_text" | "input_text")) {
                            if let Some(value) = item.text.as_deref() {
                                text.push_str(value);
                            }
                        }
                    }
                }
                push_block(&mut p.blocks, "assistant", text);
            }
            _ => {}
        }
    }
    p.title = title_from(&p.blocks);
    p
}

pub(super) fn scan_codex_summary(path: &Path) -> Option<ParsedSummary> {
    cached_summary(codex_summary_cache(), path, scan_codex_summary_uncached)
}

pub(super) fn scan_codex_summary_uncached(path: &Path) -> Option<ParsedSummary> {
    let size = fs::metadata(path).map(|m| m.len()).ok()?;
    if size == 0 {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut cwd = String::new();
    let mut first_user = String::new();
    let mut created = None;
    let mut updated = None;
    let mut message_count = 0usize;
    let mut stable_id = None;

    let mut line = String::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            break;
        };
        if read == 0 {
            break;
        }
        let Ok(event) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        if let Some(ts) = event.timestamp.as_deref().and_then(iso_secs) {
            updated = Some(ts);
            if created.is_none() {
                created = Some(ts);
            }
        }

        let ty = event.kind.as_deref().unwrap_or_default();
        let payload = event.payload.as_ref();
        let pty = payload
            .and_then(|value| value.kind.as_deref())
            .unwrap_or_default();
        match (ty, pty) {
            ("session_meta", _) => {
                if cwd.is_empty() {
                    if let Some(value) = payload.and_then(|value| value.cwd.as_deref()) {
                        cwd = value.to_string();
                    }
                }
                if stable_id.is_none() {
                    stable_id = payload
                        .and_then(|value| value.id.as_deref().or(value.session_id.as_deref()))
                        .and_then(extract_uuid);
                }
            }
            ("event_msg", "user_message") => {
                if let Some(m) = payload.and_then(|value| value.message.as_deref()) {
                    message_count += 1;
                    if first_user.is_empty() {
                        first_user = m.to_string();
                    }
                }
            }
            ("response_item", "message")
                if payload.and_then(|value| value.role.as_deref()) == Some("assistant") =>
            {
                message_count += 1;
            }
            _ => {}
        }
    }

    if cwd.is_empty() && message_count == 0 {
        return None;
    }

    Some(ParsedSummary {
        stable_id: stable_id.or_else(|| codex_id_from_filename(path)),
        cwd,
        title: title_from_user_text(&first_user),
        created,
        updated,
        message_count,
    })
}

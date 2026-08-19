// ===== Codex =====

#[derive(serde::Deserialize)]
struct CodexEnvelope<'a> {
    #[serde(default, borrow)]
    timestamp: Option<std::borrow::Cow<'a, str>>,
    #[serde(rename = "type", default, borrow)]
    kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    payload: Option<CodexPayload<'a>>,
}

#[derive(serde::Deserialize)]
struct CodexPayload<'a> {
    #[serde(rename = "type", default, borrow)]
    kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    role: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    cwd: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    id: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    session_id: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    message: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    content: Vec<CodexContent<'a>>,
}

#[derive(serde::Deserialize)]
struct CodexContent<'a> {
    #[serde(rename = "type", default, borrow)]
    kind: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    text: Option<std::borrow::Cow<'a, str>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CodexFileStamp {
    len: u64,
    modified_nanos: u128,
}

#[derive(Clone)]
struct CodexSummaryCacheEntry {
    stamp: CodexFileStamp,
    summary: Option<ParsedSummary>,
}

fn codex_summary_cache() -> &'static Mutex<HashMap<PathBuf, CodexSummaryCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CodexSummaryCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn codex_cache_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn codex_file_stamp(path: &Path) -> Option<CodexFileStamp> {
    let metadata = fs::metadata(path).ok()?;
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(CodexFileStamp {
        len: metadata.len(),
        modified_nanos,
    })
}

fn prune_codex_summary_cache(live_paths: &HashSet<PathBuf>) {
    if let Ok(mut cache) = codex_summary_cache().lock() {
        cache.retain(|path, _| live_paths.contains(path));
    }
}

fn parse_codex(content: &str) -> Parsed {
    parse_codex_reader(BufReader::new(content.as_bytes()))
}

fn parse_codex_file(path: &Path) -> anyhow::Result<Parsed> {
    Ok(parse_codex_reader(BufReader::new(fs::File::open(path)?)))
}

fn load_codex_file_page(
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

fn parse_codex_reader(mut reader: impl BufRead) -> Parsed {
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

fn scan_codex_summary(path: &Path) -> Option<ParsedSummary> {
    let stamp = codex_file_stamp(path)?;
    let key = codex_cache_key(path);
    if let Ok(cache) = codex_summary_cache().lock() {
        if let Some(entry) = cache.get(&key).filter(|entry| entry.stamp == stamp) {
            return entry.summary.clone();
        }
    }

    let summary = scan_codex_summary_uncached(path);
    if let Ok(mut cache) = codex_summary_cache().lock() {
        cache.insert(
            key,
            CodexSummaryCacheEntry {
                stamp,
                summary: summary.clone(),
            },
        );
    }
    summary
}

fn scan_codex_summary_uncached(path: &Path) -> Option<ParsedSummary> {
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

#[derive(Clone, Default)]
struct CodexSessionMeta {
    id: String,
    rollout_path: String,
    cwd: String,
    title: String,
    first_user_message: String,
    created: Option<i64>,
    updated: Option<i64>,
    history: Vec<(i64, String)>,
}

fn merge_codex_meta(target: &mut CodexSessionMeta, incoming: CodexSessionMeta) {
    let incoming_is_newer = incoming.updated.unwrap_or(0) >= target.updated.unwrap_or(0);
    if target.id.is_empty() {
        target.id = incoming.id;
    }
    if !incoming.rollout_path.is_empty() && (target.rollout_path.is_empty() || incoming_is_newer) {
        target.rollout_path = incoming.rollout_path;
    }
    if !incoming.cwd.is_empty() && (target.cwd.is_empty() || incoming_is_newer) {
        target.cwd = incoming.cwd;
    }
    if !incoming.title.is_empty() && (target.title.is_empty() || incoming_is_newer) {
        target.title = incoming.title;
    }
    if !incoming.first_user_message.is_empty() && target.first_user_message.is_empty() {
        target.first_user_message = incoming.first_user_message;
    }
    target.created = [target.created, incoming.created]
        .into_iter()
        .flatten()
        .min();
    target.updated = [target.updated, incoming.updated]
        .into_iter()
        .flatten()
        .max();
    target.history.extend(incoming.history);
}

fn codex_configured_sqlite_home(codex_home: &Path) -> Option<PathBuf> {
    let content = fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let document = content.parse::<toml_edit::DocumentMut>().ok()?;
    let raw = document.get("sqlite_home")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(codex_home.join(path))
    }
}

fn codex_database_dirs(codex_home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![codex_home.to_path_buf(), codex_home.join("sqlite")];
    if let Some(path) = std::env::var_os("CODEX_SQLITE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        dirs.push(if path.is_absolute() {
            path
        } else {
            codex_home.join(path)
        });
    }
    if let Some(path) = codex_configured_sqlite_home(codex_home) {
        dirs.push(path);
    }
    let mut seen = HashSet::new();
    dirs.retain(|path| seen.insert(normalize_codex_workspace(&path.to_string_lossy())));
    dirs
}

fn codex_database_paths_all(codex_home: &Path, prefix: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for dir in codex_database_dirs(codex_home) {
        match fs::metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => out.extend(codex_database_paths(&dir, prefix)?),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn sqlite_optional_text(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("coalesce(\"{}\", '')", name.replace('"', "\"\""))
    } else {
        "''".to_string()
    }
}

fn sqlite_optional_integer(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        "null".to_string()
    }
}

fn sqlite_optional_epoch(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!(
            "cast(\"{}\" as integer)",
            name.replace('"', "\"\"")
        )
    } else {
        "null".to_string()
    }
}

fn codex_state_metadata_rows(db_path: &Path) -> anyhow::Result<Vec<CodexSessionMeta>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    if !sqlite_table_names(&conn)?
        .iter()
        .any(|name| name == "threads")
    {
        return Ok(Vec::new());
    }
    let columns = sqlite_table_columns(&conn, "threads")?;
    if !columns.iter().any(|column| column == "id") {
        return Ok(Vec::new());
    }
    let fields = [
        sqlite_optional_text(&columns, "id"),
        sqlite_optional_text(&columns, "rollout_path"),
        sqlite_optional_text(&columns, "cwd"),
        sqlite_optional_text(&columns, "name"),
        sqlite_optional_text(&columns, "title"),
        sqlite_optional_text(&columns, "first_user_message"),
        sqlite_optional_text(&columns, "preview"),
        sqlite_optional_integer(&columns, "created_at_ms"),
        sqlite_optional_integer(&columns, "created_at"),
        sqlite_optional_integer(&columns, "updated_at_ms"),
        sqlite_optional_integer(&columns, "updated_at"),
        sqlite_optional_integer(&columns, "recency_at_ms"),
        sqlite_optional_integer(&columns, "recency_at"),
    ];
    let mut stmt = conn.prepare(&format!("select {} from threads", fields.join(", ")))?;
    let rows = stmt.query_map([], |row| {
        let name = row.get::<_, String>(3)?;
        let title = row.get::<_, String>(4)?;
        let first_user_message = row.get::<_, String>(5)?;
        let preview = row.get::<_, String>(6)?;
        let created_ms = row.get::<_, Option<i64>>(7)?;
        let created = row.get::<_, Option<i64>>(8)?;
        let updated_ms = row.get::<_, Option<i64>>(9)?;
        let updated = row.get::<_, Option<i64>>(10)?;
        let recency_ms = row.get::<_, Option<i64>>(11)?;
        let recency = row.get::<_, Option<i64>>(12)?;
        Ok(CodexSessionMeta {
            id: row.get(0)?,
            rollout_path: row.get(1)?,
            cwd: row.get(2)?,
            title: [name, title, preview]
                .into_iter()
                .find(|value| !value.trim().is_empty())
                .unwrap_or_default(),
            first_user_message,
            created: created_ms
                .filter(|value| *value > 0)
                .or_else(|| created.filter(|value| *value > 0))
                .map(epoch_secs),
            updated: recency_ms
                .filter(|value| *value > 0)
                .or_else(|| updated_ms.filter(|value| *value > 0))
                .or_else(|| recency.filter(|value| *value > 0))
                .or_else(|| updated.filter(|value| *value > 0))
                .map(epoch_secs),
            history: Vec::new(),
        })
    })?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|row| !row.id.is_empty())
        .collect())
}

fn codex_catalog_database_paths_all(codex_home: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for dir in codex_database_dirs(codex_home) {
        match fs::metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if name.starts_with("codex") && (name.ends_with(".db") || name.ends_with(".sqlite")) {
                out.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn codex_catalog_metadata_rows(db_path: &Path) -> anyhow::Result<Vec<CodexSessionMeta>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    if !sqlite_table_names(&conn)?
        .iter()
        .any(|name| name == "local_thread_catalog")
    {
        return Ok(Vec::new());
    }
    let columns = sqlite_table_columns(&conn, "local_thread_catalog")?;
    if !columns.iter().any(|column| column == "thread_id") {
        return Ok(Vec::new());
    }
    let fields = [
        sqlite_optional_text(&columns, "thread_id"),
        sqlite_optional_text(&columns, "display_title"),
        sqlite_optional_text(&columns, "cwd"),
        sqlite_optional_epoch(&columns, "source_created_at"),
        sqlite_optional_epoch(&columns, "source_updated_at"),
        sqlite_optional_epoch(&columns, "source_recency_at"),
    ];
    let mut stmt = conn.prepare(&format!(
        "select {} from local_thread_catalog",
        fields.join(", ")
    ))?;
    let rows = stmt.query_map([], |row| {
        let created = row.get::<_, Option<i64>>(3)?;
        let updated = row.get::<_, Option<i64>>(4)?;
        let recency = row.get::<_, Option<i64>>(5)?;
        Ok(CodexSessionMeta {
            id: row.get(0)?,
            title: row.get(1)?,
            cwd: row.get(2)?,
            created: created.filter(|value| *value > 0).map(epoch_secs),
            updated: recency
                .filter(|value| *value > 0)
                .or_else(|| updated.filter(|value| *value > 0))
                .map(epoch_secs),
            ..Default::default()
        })
    })?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|row| !row.id.is_empty())
        .collect())
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CodexTimestamp {
    String(String),
    Integer(i64),
    Float(f64),
}

impl CodexTimestamp {
    fn into_secs(self) -> Option<i64> {
        match self {
            Self::String(value) => iso_secs(&value).or_else(|| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .map(|value| epoch_secs(value as i64))
            }),
            Self::Integer(value) => Some(epoch_secs(value)),
            Self::Float(value) if value.is_finite() => Some(epoch_secs(value as i64)),
            Self::Float(_) => None,
        }
    }
}

#[derive(serde::Deserialize)]
struct CodexSessionIndexEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    thread_name: Option<String>,
    #[serde(default)]
    updated_at: Option<CodexTimestamp>,
}

#[derive(serde::Deserialize)]
struct CodexHistoryEntry {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    ts: Option<CodexTimestamp>,
}

fn codex_metadata_result(root: &Path) -> anyhow::Result<HashMap<String, CodexSessionMeta>> {
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut out: HashMap<String, CodexSessionMeta> = HashMap::new();

    for db_path in codex_database_paths_all(codex_home, "state_")? {
        let Ok(rows) = codex_state_metadata_rows(&db_path) else {
            continue;
        };
        for mut row in rows {
            let Some(id) = extract_uuid(&row.id) else {
                continue;
            };
            row.id = id.clone();
            merge_codex_meta(out.entry(id).or_default(), row);
        }
    }

    for db_path in codex_catalog_database_paths_all(codex_home)? {
        let Ok(rows) = codex_catalog_metadata_rows(&db_path) else {
            continue;
        };
        for mut row in rows {
            let Some(id) = extract_uuid(&row.id) else {
                continue;
            };
            row.id = id.clone();
            merge_codex_meta(out.entry(id).or_default(), row);
        }
    }

    for name in ["session_index.jsonl", "session_index.jsonl.bak"] {
        let is_backup = name.ends_with(".bak");
        let path = codex_home.join(name);
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<CodexSessionIndexEntry>(&line) else {
                continue;
            };
            let Some(id) = value
                .id
                .as_deref()
                .and_then(extract_uuid)
            else {
                continue;
            };
            let entry = out.entry(id.clone()).or_insert_with(|| CodexSessionMeta {
                id,
                ..Default::default()
            });
            if let Some(title) = value.thread_name.as_deref() {
                if !title.trim().is_empty() && (!is_backup || entry.title.trim().is_empty()) {
                    entry.title = title.to_string();
                }
            }
            if let Some(updated) = value.updated_at.and_then(CodexTimestamp::into_secs) {
                entry.updated = [entry.updated, Some(updated)].into_iter().flatten().max();
            }
        }
    }

    for name in ["history.jsonl", "history.jsonl.bak"] {
        let Ok(file) = fs::File::open(codex_home.join(name)) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<CodexHistoryEntry>(&line) else {
                continue;
            };
            let Some(id) = value
                .session_id
                .as_deref()
                .and_then(extract_uuid)
            else {
                continue;
            };
            let text = value
                .text
                .as_deref()
                .unwrap_or_default()
                .trim();
            if text.is_empty() {
                continue;
            }
            let timestamp = value
                .ts
                .and_then(CodexTimestamp::into_secs)
                .unwrap_or_default();
            let entry = out.entry(id.clone()).or_insert_with(|| CodexSessionMeta {
                id,
                ..Default::default()
            });
            entry.history.push((timestamp, text.to_string()));
            if entry.first_user_message.is_empty() {
                entry.first_user_message = text.to_string();
            }
            if timestamp > 0 {
                entry.created = [entry.created, Some(timestamp)].into_iter().flatten().min();
                entry.updated = [entry.updated, Some(timestamp)].into_iter().flatten().max();
            }
        }
    }

    for entry in out.values_mut() {
        entry.history.sort_by_key(|(timestamp, _)| *timestamp);
        entry.history.dedup();
    }
    Ok(out)
}

fn codex_meta_title(meta: &CodexSessionMeta) -> String {
    let value = if !meta.title.trim().is_empty() {
        meta.title.as_str()
    } else if !meta.first_user_message.trim().is_empty() {
        meta.first_user_message.as_str()
    } else {
        meta.history
            .first()
            .map(|(_, text)| text.as_str())
            .unwrap_or_default()
    };
    title_from_user_text(value)
}

fn merge_codex_summary_metadata(
    d: &SourceDef,
    summary: &mut SessionSummary,
    meta: CodexSessionMeta,
) {
    if !meta.title.trim().is_empty() {
        summary.title = codex_meta_title(&meta);
    }
    if summary.workspace_directory.is_empty() && !meta.cwd.is_empty() {
        summary.workspace_directory = meta.cwd.clone();
        summary.workspace_hash = format!("{}{}", d.prefix, meta.cwd);
    }
    summary.created_at = [summary.created_at, meta.created]
        .into_iter()
        .flatten()
        .min();
    summary.modified_at = [summary.modified_at, meta.updated]
        .into_iter()
        .flatten()
        .max();
    summary.message_count = summary.message_count.max(meta.history.len());
}

fn append_codex_file_summary(
    d: &SourceDef,
    file: &Path,
    session_id: String,
    metadata: &mut HashMap<String, CodexSessionMeta>,
    seen_ids: &mut HashSet<String>,
    out: &mut Vec<SessionSummary>,
) {
    let Some(parsed) = scan_codex_summary(file) else {
        return;
    };
    let stable_id = parsed.stable_id.clone();
    if stable_id
        .as_ref()
        .is_some_and(|id| !seen_ids.insert(id.clone()))
    {
        return;
    }
    let mut summary = summary_from_scan(d, parsed, session_id, file);
    if let Some(meta) = stable_id.as_ref().and_then(|id| metadata.remove(id)) {
        merge_codex_summary_metadata(d, &mut summary, meta);
    }
    if summary.workspace_directory.trim().is_empty() {
        let fallback = if virtual_session_key(&summary.session_id, "archive").is_some() {
            "Codex 归档"
        } else {
            "Codex 历史"
        };
        summary.workspace_directory = fallback.to_string();
        summary.workspace_hash = format!("{}{fallback}", d.prefix);
    }
    out.push(summary);
}

fn codex_archive_root(root: &Path) -> Option<PathBuf> {
    root.parent().map(|home| home.join("archived_sessions"))
}

fn codex_archive_session_id(root: &Path, file: &Path) -> Option<String> {
    let key = rel_key(root, file)?;
    Some(virtual_session_id(
        "archive",
        &urlencoding::encode(&key),
    ))
}

fn codex_archive_session_path(root: &Path, session_id: &str) -> Option<PathBuf> {
    let encoded = virtual_session_key(session_id, "archive")?;
    let relative = urlencoding::decode(encoded).ok()?.into_owned();
    let archive_root = codex_archive_root(root)?;
    safe_path(&archive_root, &relative)
}

fn collect_codex_source(d: &SourceDef, root: &Path, depth: usize) -> Vec<SessionSummary> {
    let mut files = Vec::new();
    collect_files(root, "jsonl", depth, &mut files);
    files.sort();
    let archive_root = codex_archive_root(root).filter(|path| path.is_dir());
    let mut archived_files = Vec::new();
    if let Some(archive_root) = archive_root.as_ref() {
        collect_files(archive_root, "jsonl", depth, &mut archived_files);
        archived_files.sort();
    }
    let live_paths = files
        .iter()
        .chain(archived_files.iter())
        .map(|path| codex_cache_key(path))
        .collect::<HashSet<_>>();
    prune_codex_summary_cache(&live_paths);

    let mut metadata = codex_metadata_result(root).unwrap_or_default();
    let mut seen_ids = HashSet::new();
    let mut out = Vec::new();

    for file in files {
        let Some(key) = rel_key(root, &file) else {
            continue;
        };
        append_codex_file_summary(
            d,
            &file,
            key,
            &mut metadata,
            &mut seen_ids,
            &mut out,
        );
    }

    if let Some(archive_root) = archive_root.as_ref() {
        for file in archived_files {
            let Some(session_id) = codex_archive_session_id(archive_root, &file) else {
                continue;
            };
            append_codex_file_summary(
                d,
                &file,
                session_id,
                &mut metadata,
                &mut seen_ids,
                &mut out,
            );
        }
    }

    for meta in metadata.into_values() {
        if seen_ids.contains(&meta.id) {
            continue;
        }
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史".to_string()
        } else {
            meta.cwd.clone()
        };
        out.push(SessionSummary {
            session_id: virtual_session_id("index", &meta.id),
            title: codex_meta_title(&meta),
            session_type: d.source.to_string(),
            workspace_directory: cwd.clone(),
            workspace_hash: format!("{}{cwd}", d.prefix),
            message_count: meta
                .history
                .len()
                .max(usize::from(!meta.first_user_message.trim().is_empty())),
            file_size: 0,
            created_at: meta.created,
            modified_at: meta.updated,
            source: d.source.to_string(),
        });
    }
    out
}

fn load_codex_virtual_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    if virtual_session_key(session_id, "archive").is_some() {
        return load_codex_archive_session(d, root, session_id);
    }
    load_codex_index_session(d, root, session_id, hash)
}

fn load_codex_virtual_session_page(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    if virtual_session_key(session_id, "archive").is_some() {
        return load_codex_archive_session_page(d, root, session_id, page, page_size);
    }
    load_codex_index_session_page(d, root, session_id, hash, page, page_size)
}

fn load_codex_archive_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
) -> anyhow::Result<IdeSession> {
    let path = codex_archive_session_path(root, session_id)
        .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
    let mut parsed = parse_codex_file(&path)?;
    if let Some(id) = codex_id_from_path(&path)? {
        if let Some(meta) = codex_metadata_result(root)?.remove(&id) {
            if parsed.cwd.is_empty() && !meta.cwd.is_empty() {
                parsed.cwd = meta.cwd.clone();
            }
            if !meta.title.trim().is_empty()
                || !meta.first_user_message.trim().is_empty()
                || !meta.history.is_empty()
            {
                parsed.title = codex_meta_title(&meta);
            }
        }
    }
    if parsed.cwd.trim().is_empty() {
        parsed.cwd = "Codex 归档".to_string();
    }
    Ok(parsed_to_session(d, session_id, parsed))
}

fn load_codex_archive_session_page(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let path = codex_archive_session_path(root, session_id)
        .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
    let metadata = if let Some(id) = codex_id_from_path(&path)? {
        codex_metadata_result(root)?.remove(&id)
    } else {
        None
    };
    let cwd_fallback = metadata
        .as_ref()
        .and_then(|meta| (!meta.cwd.is_empty()).then(|| meta.cwd.clone()))
        .or_else(|| Some("Codex 归档".to_string()));
    let title_override = metadata.as_ref().and_then(|meta| {
        (!meta.title.trim().is_empty()
            || !meta.first_user_message.trim().is_empty()
            || !meta.history.is_empty())
        .then(|| codex_meta_title(meta))
    });
    load_codex_file_page(
        d,
        &path,
        session_id,
        page,
        page_size,
        cwd_fallback,
        title_override,
    )
}

fn load_codex_index_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let raw_id = virtual_session_key(session_id, "index")
        .ok_or_else(|| anyhow::anyhow!("无法识别 Codex 索引会话"))?;
    let id = extract_uuid(raw_id).ok_or_else(|| anyhow::anyhow!("Codex 会话 ID 无效"))?;
    let meta = codex_metadata_result(root)?
        .remove(&id)
        .ok_or_else(|| anyhow::anyhow!("Codex 索引记录已不存在，请刷新列表"))?;
    if let Some(path) = safe_codex_rollout_path(root, &meta.rollout_path) {
        let mut parsed = parse_codex_file(&path)?;
        if parsed.cwd.is_empty() {
            parsed.cwd = meta.cwd.clone();
        }
        if !meta.title.trim().is_empty() {
            parsed.title = codex_meta_title(&meta);
        }
        return Ok(parsed_to_session(d, session_id, parsed));
    }

    Ok(codex_index_fallback_session(d, session_id, hash, meta))
}

fn load_codex_index_session_page(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let raw_id = virtual_session_key(session_id, "index")
        .ok_or_else(|| anyhow::anyhow!("无法识别 Codex 索引会话"))?;
    let id = extract_uuid(raw_id).ok_or_else(|| anyhow::anyhow!("Codex 会话 ID 无效"))?;
    let meta = codex_metadata_result(root)?
        .remove(&id)
        .ok_or_else(|| anyhow::anyhow!("Codex 索引记录已不存在，请刷新列表"))?;
    if let Some(path) = safe_codex_rollout_path(root, &meta.rollout_path) {
        let cwd_fallback = (!meta.cwd.is_empty()).then(|| meta.cwd.clone());
        let title_override = (!meta.title.trim().is_empty()).then(|| codex_meta_title(&meta));
        return load_codex_file_page(
            d,
            &path,
            session_id,
            page,
            page_size,
            cwd_fallback,
            title_override,
        );
    }
    Ok(SessionPage::from_session(
        codex_index_fallback_session(d, session_id, hash, meta),
        page,
        page_size,
    ))
}

fn codex_index_fallback_session(
    d: &SourceDef,
    session_id: &str,
    hash: &str,
    meta: CodexSessionMeta,
) -> IdeSession {
    let cwd = if meta.cwd.trim().is_empty() {
        hash.strip_prefix(d.prefix)
            .unwrap_or("Codex 历史")
            .to_string()
    } else {
        meta.cwd.clone()
    };
    let mut history = vec![history_item(
        "system",
        "Codex 正文文件已不在磁盘；以下内容由 SQLite、会话索引和 history.jsonl 恢复，通常只包含用户输入。".to_string(),
        0,
    )];
    for (_, text) in &meta.history {
        history.push(history_item("user", text.clone(), history.len()));
    }
    if history.len() == 1 && !meta.first_user_message.trim().is_empty() {
        history.push(history_item(
            "user",
            meta.first_user_message.clone(),
            history.len(),
        ));
    }
    IdeSession {
        session_id: session_id.to_string(),
        title: codex_meta_title(&meta),
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history,
        conversation_summary: None,
    }
}

fn normalize_codex_workspace(value: &str) -> String {
    let value = value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| value.strip_prefix(r"\\?\").unwrap_or(value).to_string());
    value
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn codex_session_identity(path: &Path) -> anyhow::Result<(Option<String>, Option<String>)> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let Ok(value) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        if value.kind.as_deref() != Some("session_meta") {
            continue;
        }
        let payload = value.payload.as_ref();
        let id = payload
            .and_then(|value| value.id.as_deref().or(value.session_id.as_deref()))
            .and_then(extract_uuid);
        let cwd = payload
            .and_then(|value| value.cwd.as_deref())
            .map(str::to_string);
        return Ok((id, cwd));
    }
    Ok((None, None))
}

fn codex_id_from_filename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(extract_uuid)
}

fn codex_id_from_path(path: &Path) -> anyhow::Result<Option<String>> {
    Ok(codex_session_identity(path)?
        .0
        .or_else(|| codex_id_from_filename(path)))
}

fn codex_database_paths(codex_home: &Path, prefix: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(codex_home)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".sqlite"))
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn collect_codex_files_strict(
    dir: &Path,
    ext: &str,
    depth: usize,
    out: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_codex_files_strict(&path, ext, depth - 1, out)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some(ext)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn sqlite_table_names(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "select name from sqlite_master where type = 'table' and name not like 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn sqlite_table_columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let quoted = table.replace('"', "\"\"");
    let mut stmt = conn.prepare(&format!("pragma table_info(\"{quoted}\")"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn codex_state_rows(db_path: &Path) -> anyhow::Result<Vec<(String, String, String)>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    if !sqlite_table_names(&conn)?
        .iter()
        .any(|name| name == "threads")
    {
        return Ok(Vec::new());
    }
    let columns = sqlite_table_columns(&conn, "threads")?;
    if !["id", "rollout_path", "cwd"]
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
    {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare("select id, rollout_path, cwd from threads")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn safe_codex_rollout_path(root: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string());
    if raw.trim().is_empty() {
        return None;
    }
    let raw_path = PathBuf::from(&raw);
    let codex_home = root.parent()?;
    let archive_root = codex_archive_root(root);
    let candidates = if raw_path.is_absolute() {
        vec![raw_path]
    } else {
        let mut values = vec![root.join(&raw_path), codex_home.join(&raw_path)];
        if let Some(archive_root) = archive_root.as_ref() {
            values.push(archive_root.join(&raw_path));
        }
        values
    };
    let mut trusted_roots = vec![root.canonicalize().ok()?];
    if let Some(archive_root) = archive_root {
        if let Ok(path) = archive_root.canonicalize() {
            trusted_roots.push(path);
        }
    }
    candidates.into_iter().find_map(|candidate| {
        if !candidate.is_file() {
            return None;
        }
        let canonical = candidate.canonicalize().ok()?;
        trusted_roots
            .iter()
            .any(|trusted| canonical.starts_with(trusted))
            .then_some(canonical)
    })
}

fn rewrite_jsonl_without_ids(
    path: &Path,
    id_field: &str,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Codex 索引路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let content = fs::read_to_string(path)?;
    let mut filtered = String::with_capacity(content.len());
    let mut changed = false;

    for line in content.split_inclusive('\n') {
        let json = line.trim_end_matches(['\r', '\n']);
        let remove = serde_json::from_str::<serde_json::Value>(json)
            .ok()
            .and_then(|value| {
                value
                    .get(id_field)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|id| ids.contains(&id));
        if remove {
            changed = true;
        } else {
            filtered.push_str(line);
        }
    }

    if changed {
        crate::ai::backend::utils::fs::atomic_write(path, &filtered, "Codex 会话索引")
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn delete_thread_rows_from_db(db_path: &Path, ids: &HashSet<String>) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let mut table_columns = sqlite_table_names(&conn)?
        .into_iter()
        .map(|table| {
            let columns = sqlite_table_columns(&conn, &table)?;
            Ok((table, columns))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    // 先删关联表，再删 threads 主表；兼容未来新增的非级联外键。
    table_columns.sort_by(|left, right| {
        (left.0 == "threads", &left.0).cmp(&(right.0 == "threads", &right.0))
    });
    let mut sorted_ids = ids.iter().collect::<Vec<_>>();
    sorted_ids.sort();

    let transaction = conn.transaction()?;
    for (table, columns) in table_columns {
        let id_columns = columns
            .iter()
            .filter(|column| {
                column.as_str() == "thread_id"
                    || column.as_str() == "parent_thread_id"
                    || column.as_str() == "child_thread_id"
                    || (table == "threads" && column.as_str() == "id")
            })
            .cloned()
            .collect::<Vec<_>>();
        if id_columns.is_empty() {
            continue;
        }
        let table = table.replace('"', "\"\"");
        for column in id_columns {
            let column = column.replace('"', "\"\"");
            // 避免大型 logs 数据库按每个 ID 重复全表扫描。
            for chunk in sorted_ids.chunks(500) {
                let placeholders = std::iter::repeat("?")
                    .take(chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!("delete from \"{table}\" where \"{column}\" in ({placeholders})");
                transaction.execute(&sql, rusqlite::params_from_iter(chunk.iter().copied()))?;
            }
        }
    }
    transaction.commit()?;
    let _ = conn.execute_batch("pragma wal_checkpoint(truncate)");
    Ok(())
}

fn cleanup_empty_codex_dirs(root: &Path, path: &Path) {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        let empty = fs::read_dir(dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty || fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

fn delete_codex_targets(
    root: &Path,
    ids: &HashSet<String>,
    paths: &HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut metadata_errors = Vec::new();

    for prefix in ["state_", "logs_", "goals_", "memories_"] {
        match codex_database_paths_all(codex_home, prefix) {
            Ok(paths) => {
                for db_path in paths {
                    if let Err(error) = delete_thread_rows_from_db(&db_path, ids) {
                        metadata_errors.push(format!("清理 {} 失败: {error}", db_path.display()));
                    }
                }
            }
            Err(error) => metadata_errors.push(format!("枚举 Codex {prefix} 数据库失败: {error}")),
        }
    }

    match codex_catalog_database_paths_all(codex_home) {
        Ok(paths) => {
            for db_path in paths {
                if let Err(error) = delete_thread_rows_from_db(&db_path, ids) {
                    metadata_errors.push(format!("清理 {} 失败: {error}", db_path.display()));
                }
            }
        }
        Err(error) => metadata_errors.push(format!("枚举 Codex 本地会话目录数据库失败: {error}")),
    }

    for (name, field) in [
        ("session_index.jsonl", "id"),
        ("session_index.jsonl.bak", "id"),
        ("history.jsonl", "session_id"),
        ("history.jsonl.bak", "session_id"),
    ] {
        let path = codex_home.join(name);
        if let Err(error) = rewrite_jsonl_without_ids(&path, field, ids) {
            metadata_errors.push(format!("清理 {} 失败: {error}", path.display()));
        }
    }

    // 元数据清理失败时保留正文文件，确保用户关闭 Codex 后还能从 SysPulse 重试。
    if !metadata_errors.is_empty() {
        return Err(anyhow::anyhow!(metadata_errors.join("；")));
    }

    let mut canonical_roots = vec![root.canonicalize()?];
    if let Some(archive_root) = codex_archive_root(root) {
        if archive_root.is_dir() {
            canonical_roots.push(archive_root.canonicalize()?);
        }
    }
    let mut file_errors = Vec::new();
    for path in paths {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                file_errors.push(format!("Codex 会话路径不是文件: {}", path.display()));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                file_errors.push(format!("读取 {} 失败: {error}", path.display()));
                continue;
            }
        }
        let (canonical, trusted_root) = match path.canonicalize() {
            Ok(path) if path.is_file() => {
                let trusted_root = canonical_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned();
                match trusted_root {
                    Some(root) => (path, root),
                    None => {
                        file_errors.push(format!(
                            "拒绝删除 Codex 会话目录之外的路径: {}",
                            path.display()
                        ));
                        continue;
                    }
                }
            }
            Ok(_) => {
                file_errors.push(format!(
                    "Codex 会话路径不是文件: {}",
                    path.display()
                ));
                continue;
            }
            Err(error) => {
                file_errors.push(format!("校验 {} 失败: {error}", path.display()));
                continue;
            }
        };
        match fs::remove_file(&canonical) {
            Ok(()) => cleanup_empty_codex_dirs(&trusted_root, &canonical),
            Err(error) => file_errors.push(format!("删除 {} 失败: {error}", path.display())),
        }
    }

    if file_errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(file_errors.join("；")))
    }
}

fn delete_codex_virtual_session(root: &Path, session_id: &str) -> anyhow::Result<()> {
    if let Some(raw_id) = virtual_session_key(session_id, "index") {
        let id = extract_uuid(raw_id)
            .ok_or_else(|| anyhow::anyhow!("Codex 索引会话 ID 无效，未执行清理"))?;
        let ids = HashSet::from([id.clone()]);
        delete_codex_targets(root, &ids, &HashSet::new())?;
        if codex_metadata_result(root)?.contains_key(&id) {
            return Err(anyhow::anyhow!(
                "Codex 索引仍有残留记录，请关闭 Codex 后重试"
            ));
        }
        return Ok(());
    }

    if virtual_session_key(session_id, "archive").is_some() {
        let path = codex_archive_session_path(root, session_id)
            .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
        return delete_codex_session(root, &path);
    }

    Err(anyhow::anyhow!("不支持的 Codex 虚拟会话类型"))
}

fn delete_codex_session(root: &Path, path: &Path) -> anyhow::Result<()> {
    let id = codex_id_from_path(path)?
        .ok_or_else(|| anyhow::anyhow!("无法识别 Codex 会话 ID，未执行不完整删除"))?;
    let ids = HashSet::from([id]);
    let paths = HashSet::from([path.to_path_buf()]);
    delete_codex_targets(root, &ids, &paths)
}

fn delete_codex_workspace(root: &Path, workspace: &str) -> anyhow::Result<()> {
    let target = normalize_codex_workspace(workspace);
    if target.is_empty() {
        return Err(anyhow::anyhow!("Codex 工作区路径为空，已拒绝删除"));
    }
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    let mut unidentified_paths = Vec::new();

    let mut rollout_roots = vec![root.to_path_buf()];
    if let Some(archive_root) = codex_archive_root(root) {
        if archive_root.is_dir() {
            rollout_roots.push(archive_root);
        }
    }
    for rollout_root in &rollout_roots {
        let mut files = Vec::new();
        collect_codex_files_strict(rollout_root, "jsonl", 8, &mut files)?;
        for path in files {
            let (id, cwd) = codex_session_identity(&path)?;
            if cwd
                .as_deref()
                .is_some_and(|cwd| normalize_codex_workspace(cwd) == target)
            {
                match id.or_else(|| codex_id_from_filename(&path)) {
                    Some(id) => {
                        ids.insert(id);
                        paths.insert(path);
                    }
                    None => unidentified_paths.push(path),
                }
            }
        }
    }

    if !unidentified_paths.is_empty() {
        let paths = unidentified_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("、");
        return Err(anyhow::anyhow!(
            "以下 Codex 会话无法识别 ID，未执行不完整删除: {paths}"
        ));
    }

    for db_path in codex_database_paths_all(codex_home, "state_")? {
        for (id, rollout_path, cwd) in codex_state_rows(&db_path)? {
            if normalize_codex_workspace(&cwd) != target {
                continue;
            }
            ids.insert(id);
            if let Some(path) = safe_codex_rollout_path(root, &rollout_path) {
                paths.insert(path);
            }
        }
    }

    for meta in codex_metadata_result(root)?.into_values() {
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史"
        } else {
            meta.cwd.as_str()
        };
        if normalize_codex_workspace(cwd) != target {
            continue;
        }
        ids.insert(meta.id);
        if let Some(path) = safe_codex_rollout_path(root, &meta.rollout_path) {
            paths.insert(path);
        }
    }

    delete_codex_targets(root, &ids, &paths)?;

    let mut file_remains = false;
    for rollout_root in &rollout_roots {
        let mut remaining_files = Vec::new();
        collect_codex_files_strict(rollout_root, "jsonl", 8, &mut remaining_files)?;
        for path in &remaining_files {
            if codex_session_identity(path)?
                .1
                .as_deref()
                .is_some_and(|cwd| normalize_codex_workspace(cwd) == target)
            {
                file_remains = true;
                break;
            }
        }
        if file_remains {
            break;
        }
    }
    let metadata_remains = codex_metadata_result(root)?.into_values().any(|meta| {
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史"
        } else {
            meta.cwd.as_str()
        };
        normalize_codex_workspace(cwd) == target
    });
    if file_remains || metadata_remains {
        return Err(anyhow::anyhow!(
            "Codex 工作区仍有残留记录，请关闭 Codex 后重试"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod codex_delete_tests {
    use super::*;

    fn no_test_root() -> Option<PathBuf> {
        None
    }

    fn test_source() -> SourceDef {
        SourceDef {
            prefix: "codex-test:",
            source: "codex",
            root: no_test_root,
            layout: Layout::File { depth: 8 },
            parse: parse_codex,
            scan: scan_codex_summary,
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_rollout(path: &Path, id: &str, cwd: &str, message: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let lines = [
            serde_json::json!({
                "timestamp": "2026-08-17T01:00:00Z",
                "type": "session_meta",
                "payload": { "id": id, "cwd": cwd }
            })
            .to_string(),
            serde_json::json!({
                "timestamp": "2026-08-17T01:00:01Z",
                "type": "event_msg",
                "payload": { "type": "user_message", "message": message }
            })
            .to_string(),
        ];
        fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn typed_codex_parser_skips_large_irrelevant_payloads() {
        let huge = "x".repeat(256 * 1024);
        let content = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({
                "timestamp": "2026-08-17T01:00:00Z",
                "type": "session_meta",
                "payload": { "id": "11111111-1111-4111-8111-111111111111", "cwd": "C:\\Work" }
            }),
            serde_json::json!({
                "timestamp": "2026-08-17T01:00:01Z",
                "type": "response_item",
                "payload": { "type": "custom_tool_call_output", "output": huge }
            }),
            serde_json::json!({
                "timestamp": "2026-08-17T01:00:02Z",
                "type": "event_msg",
                "payload": { "type": "user_message", "message": "保留这条用户消息" }
            })
        );
        let parsed = parse_codex(&content);
        assert_eq!(parsed.cwd, "C:\\Work");
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].1, "保留这条用户消息");
    }

    #[test]
    fn streams_codex_page_without_losing_workspace_offset() {
        let dir = test_dir("codex-page");
        let path = dir.join("rollout.jsonl");
        let lines = [
            serde_json::json!({
                "type": "session_meta",
                "payload": { "cwd": "C:\\Workspace\\Paged" }
            }),
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "user_message", "message": "第一条" }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "第二条" }]
                }
            }),
            serde_json::json!({
                "type": "event_msg",
                "payload": { "type": "user_message", "message": "第三条" }
            }),
            serde_json::json!({
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "第四条" }]
                }
            }),
        ];
        fs::write(
            &path,
            lines
                .iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();

        let page = load_codex_file_page(
            &test_source(),
            &path,
            "session",
            2,
            2,
            None,
            None,
        )
        .unwrap();
        assert_eq!(page.total_messages, 5);
        assert_eq!(page.title, "第一条");
        assert_eq!(page.history.len(), 2);
        assert_eq!(page.history[0].message.content[0].text, "第二条");
        assert_eq!(page.history[1].message.content[0].text, "第三条");

        let first = load_codex_file_page(
            &test_source(),
            &path,
            "session",
            1,
            2,
            None,
            None,
        )
        .unwrap();
        assert_eq!(first.history.len(), 2);
        assert_eq!(first.history[0].message.role, "system");
        assert_eq!(first.history[1].message.content[0].text, "第一条");

        let out_of_range = load_codex_file_page(
            &test_source(),
            &path,
            "session",
            4,
            2,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out_of_range.total_messages, 5);
        assert!(out_of_range.history.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn codex_summary_cache_invalidates_when_file_changes() {
        let dir = test_dir("codex-cache");
        let id = "11111111-1111-4111-8111-111111111111";
        let path = dir.join(format!("rollout-{id}.jsonl"));
        write_rollout(&path, id, "C:\\Work", "第一次标题");
        let first = scan_codex_summary(&path).unwrap();
        let cached = scan_codex_summary(&path).unwrap();
        assert_eq!(cached.title, first.title);

        write_rollout(&path, id, "C:\\Work", "第二次标题，长度不同以触发文件戳更新");
        let changed = scan_codex_summary(&path).unwrap();
        assert_ne!(changed.title, first.title);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scans_archived_rollouts_and_prefers_active_duplicate() {
        let home = test_dir("codex-archive");
        let root = home.join("sessions");
        let archive = home.join("archived_sessions");
        let active_id = "11111111-1111-4111-8111-111111111111";
        let archive_id = "22222222-2222-4222-8222-222222222222";
        write_rollout(
            &root.join(format!("rollout-{active_id}.jsonl")),
            active_id,
            "C:\\Active",
            "当前正文优先",
        );
        write_rollout(
            &archive.join(format!("rollout-{active_id}.jsonl")),
            active_id,
            "C:\\Archive",
            "重复归档不应出现",
        );
        write_rollout(
            &archive.join(format!("rollout-{archive_id}.jsonl")),
            archive_id,
            "C:\\Archive",
            "仅归档会话",
        );

        let source = test_source();
        let summaries = collect_codex_source(&source, &root, 8);
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().any(|item| item.title == "当前正文优先"));
        assert!(!summaries.iter().any(|item| item.title == "重复归档不应出现"));
        let archived = summaries
            .iter()
            .find(|item| item.title == "仅归档会话")
            .unwrap();
        assert!(archived.session_id.starts_with("@archive/"));
        let loaded = load_codex_virtual_session(
            &source,
            &root,
            &archived.session_id,
            &archived.workspace_hash,
        )
        .unwrap();
        assert!(loaded.history.iter().any(|item| {
            item.message.content[0].text.contains("仅归档会话")
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn restores_catalog_only_thread_and_history_backup() {
        let home = test_dir("codex-catalog");
        let root = home.join("sessions");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sqlite_dir).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let db = sqlite_dir.join("codex-test.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table local_thread_catalog (\
                thread_id text, display_title text, cwd text,\
                source_created_at real, source_updated_at real, source_recency_at real\
             );",
        )
        .unwrap();
        conn.execute(
            "insert into local_thread_catalog values (?1, '旧目录标题', 'C:\\Catalog', 1700000000.0, 1700000100.0, 1700000200.0)",
            [id],
        )
        .unwrap();
        drop(conn);
        fs::write(
            home.join("history.jsonl.bak"),
            format!("{{\"session_id\":\"{id}\",\"ts\":1700000050,\"text\":\"备份历史输入\"}}\n"),
        )
        .unwrap();

        let source = test_source();
        let summaries = collect_codex_source(&source, &root, 8);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, virtual_session_id("index", id));
        assert_eq!(summaries[0].title, "旧目录标题");
        assert_eq!(summaries[0].workspace_directory, "C:\\Catalog");
        assert_eq!(summaries[0].message_count, 1);
        let loaded = load_codex_virtual_session(
            &source,
            &root,
            &summaries[0].session_id,
            &summaries[0].workspace_hash,
        )
        .unwrap();
        assert!(loaded.history.iter().any(|item| {
            item.message.content[0].text.contains("备份历史输入")
        }));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn resolves_relative_sqlite_home_from_codex_home() {
        let home = test_dir("codex-sqlite-home");
        fs::write(home.join("config.toml"), "sqlite_home = 'relative-db'\n").unwrap();
        assert_eq!(
            codex_configured_sqlite_home(&home).unwrap(),
            home.join("relative-db")
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rewrites_codex_indexes_by_session_id() {
        let dir = test_dir("codex-index");
        let path = dir.join("session_index.jsonl");
        fs::write(&path, "{\"id\":\"keep\"}\n{\"id\":\"remove\"}\nnot-json\n").unwrap();
        rewrite_jsonl_without_ids(&path, "id", &HashSet::from(["remove".to_string()])).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"id\":\"keep\"}\nnot-json\n"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deletes_catalog_only_codex_index_session() {
        let home = test_dir("codex-index-delete");
        let root = home.join("sessions");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sqlite_dir).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        fs::write(
            home.join("session_index.jsonl"),
            format!("{{\"id\":\"{id}\",\"thread_name\":\"残留索引\"}}\n"),
        )
        .unwrap();
        fs::write(
            home.join("history.jsonl"),
            format!("{{\"session_id\":\"{id}\",\"text\":\"残留输入\"}}\n"),
        )
        .unwrap();

        let db = sqlite_dir.join("codex-test.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table local_thread_catalog (thread_id text, display_title text, cwd text);",
        )
        .unwrap();
        conn.execute(
            "insert into local_thread_catalog values (?1, '残留索引', 'C:\\Catalog')",
            [id],
        )
        .unwrap();
        drop(conn);

        delete_codex_virtual_session(&root, &virtual_session_id("index", id)).unwrap();

        assert!(!fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(id));
        assert!(!fs::read_to_string(home.join("history.jsonl"))
            .unwrap()
            .contains(id));
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row(
                "select count(*) from local_thread_catalog where thread_id = ?1",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        drop(conn);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn deletes_archived_codex_session_and_metadata() {
        let home = test_dir("codex-archive-delete");
        let root = home.join("sessions");
        let archive = home.join("archived_sessions");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let rollout = archive.join("2026").join(format!("rollout-{id}.jsonl"));
        write_rollout(&rollout, id, "C:\\Archive", "归档会话");
        fs::write(
            home.join("session_index.jsonl"),
            format!("{{\"id\":\"{id}\"}}\n"),
        )
        .unwrap();
        let session_id = codex_archive_session_id(&archive, &rollout).unwrap();

        delete_codex_virtual_session(&root, &session_id).unwrap();

        assert!(!rollout.exists());
        assert!(!fs::read_to_string(home.join("session_index.jsonl"))
            .unwrap()
            .contains(id));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn deletes_catalog_only_codex_workspace() {
        let home = test_dir("codex-catalog-workspace-delete");
        let root = home.join("sessions");
        let sqlite_dir = home.join("sqlite");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sqlite_dir).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let db = sqlite_dir.join("codex-test.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table local_thread_catalog (thread_id text, display_title text, cwd text);",
        )
        .unwrap();
        conn.execute(
            "insert into local_thread_catalog values (?1, '残留索引', 'C:\\Catalog')",
            [id],
        )
        .unwrap();
        drop(conn);

        delete_codex_workspace(&root, "C:\\Catalog").unwrap();

        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row("select count(*) from local_thread_catalog", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            0
        );
        drop(conn);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn restores_codex_session_from_state_index_and_history() {
        let home = test_dir("codex-restore");
        let root = home.join("sessions");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let db = home.join("state_5.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table threads (id text, rollout_path text, cwd text, title text, created_at integer, updated_at integer, first_user_message text);",
        )
        .unwrap();
        conn.execute(
            "insert into threads values (?1, '', 'C:\\Workspace\\Demo', '数据库标题', 1700000000, 1700000100, '第一条输入')",
            [id],
        )
        .unwrap();
        drop(conn);
        fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{id}\",\"thread_name\":\"索引标题\",\"updated_at\":\"2023-11-14T22:16:00Z\"}}\n"
            ),
        )
        .unwrap();
        fs::write(
            home.join("history.jsonl"),
            format!("{{\"session_id\":\"{id}\",\"ts\":1700000050,\"text\":\"恢复的用户输入\"}}\n"),
        )
        .unwrap();

        let summaries = collect_codex_source(&test_source(), &root, 8);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, virtual_session_id("index", id));
        assert_eq!(summaries[0].title, "索引标题");
        assert_eq!(summaries[0].workspace_directory, "C:\\Workspace\\Demo");
        assert_eq!(summaries[0].message_count, 1);

        let loaded = load_codex_index_session(
            &test_source(),
            &root,
            &summaries[0].session_id,
            &summaries[0].workspace_hash,
        )
        .unwrap();
        assert_eq!(loaded.history.len(), 2);
        assert!(loaded.history[1].message.content[0]
            .text
            .contains("恢复的用户输入"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn normalizes_extended_windows_workspace_paths() {
        assert_eq!(
            normalize_codex_workspace(r"\\?\C:\Workspace\Target\"),
            r"c:\workspace\target"
        );
        assert_eq!(
            normalize_codex_workspace(r"\\?\UNC\Server\Share\Project"),
            r"\\server\share\project"
        );
    }

    #[test]
    fn rejects_empty_codex_workspace() {
        let root = test_dir("codex-empty-workspace");
        assert!(delete_codex_workspace(&root, "").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeps_unidentified_codex_rollout_for_retry() {
        let home = test_dir("codex-unidentified");
        let root = home.join("sessions");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout-without-id.jsonl");
        fs::write(
            &rollout,
            r#"{"type":"session_meta","payload":{"cwd":"C:\\Workspace\\Target"}}
"#,
        )
        .unwrap();

        assert!(delete_codex_workspace(&root, "C:\\Workspace\\Target").is_err());
        assert!(rollout.exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn removes_thread_rows_from_related_tables() {
        let dir = test_dir("codex-db");
        let path = dir.join("state_5.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "create table threads (id text primary key, rollout_path text, cwd text);\
             create table thread_spawn_edges (parent_thread_id text, child_thread_id text);\
             create table logs (thread_id text, body text);\
             insert into threads values ('remove', 'a', 'c'), ('keep', 'b', 'd');\
             insert into thread_spawn_edges values ('remove', 'keep'), ('keep', 'remove');\
             insert into logs values ('remove', 'x'), ('keep', 'y');",
        )
        .unwrap();
        drop(conn);

        delete_thread_rows_from_db(&path, &HashSet::from(["remove".to_string()])).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row(
                "select count(*) from threads where id = 'remove'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("select count(*) from thread_spawn_edges", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "select count(*) from logs where thread_id = 'remove'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        drop(conn);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deletes_workspace_files_and_orphan_codex_threads() {
        let home = test_dir("codex-workspace");
        let root = home.join("sessions");
        let day = root.join("2026").join("07").join("27");
        fs::create_dir_all(&day).unwrap();
        let removed_id = "11111111-1111-4111-8111-111111111111";
        let orphan_id = "22222222-2222-4222-8222-222222222222";
        let keep_id = "33333333-3333-4333-8333-333333333333";
        let rollout = day.join(format!("rollout-{removed_id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{removed_id}\",\"cwd\":\"C:\\\\Workspace\\\\Target\"}}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{removed_id}\"}}\n{{\"id\":\"{orphan_id}\"}}\n{{\"id\":\"{keep_id}\"}}\n"
            ),
        )
        .unwrap();
        fs::write(
            home.join("history.jsonl"),
            format!(
                "{{\"session_id\":\"{removed_id}\",\"text\":\"remove\"}}\n{{\"session_id\":\"{keep_id}\",\"text\":\"keep\"}}\n"
            ),
        )
        .unwrap();

        let db = home.join("state_5.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table threads (id text primary key, rollout_path text, cwd text);",
        )
        .unwrap();
        for (id, cwd) in [
            (removed_id, "C:\\Workspace\\Target"),
            (orphan_id, "C:\\Workspace\\Target"),
            (keep_id, "C:\\Workspace\\Other"),
        ] {
            conn.execute(
                "insert into threads (id, rollout_path, cwd) values (?1, '', ?2)",
                rusqlite::params![id, cwd],
            )
            .unwrap();
        }
        drop(conn);

        delete_codex_workspace(&root, "C:\\Workspace\\Target").unwrap();
        assert!(!rollout.exists());
        let index = fs::read_to_string(home.join("session_index.jsonl")).unwrap();
        assert!(index.contains(keep_id));
        assert!(!index.contains(removed_id));
        let history = fs::read_to_string(home.join("history.jsonl")).unwrap();
        assert!(!history.contains(removed_id));
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row(
                "select count(*) from threads where cwd = ?1",
                ["C:\\Workspace\\Target"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "select count(*) from threads where id = ?1",
                [keep_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        drop(conn);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn keeps_rollout_file_when_codex_metadata_cleanup_fails() {
        let home = test_dir("codex-retry");
        let root = home.join("sessions");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let rollout = root.join(format!("rollout-{id}.jsonl"));
        fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:\\\\Workspace\\\\Target\"}}}}\n"
            ),
        )
        .unwrap();
        // 用同名目录模拟索引被占用/不可写，元数据阶段必须失败且保留正文供重试。
        fs::create_dir(home.join("session_index.jsonl")).unwrap();

        assert!(delete_codex_session(&root, &rollout).is_err());
        assert!(rollout.exists());
        fs::remove_dir_all(home).unwrap();
    }
}

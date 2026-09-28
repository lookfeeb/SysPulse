use super::*;

#[derive(Clone, Default)]
pub(super) struct CodexSessionMeta {
    pub(super) id: String,
    pub(super) rollout_path: String,
    pub(super) cwd: String,
    pub(super) title: String,
    pub(super) first_user_message: String,
    pub(super) created: Option<i64>,
    pub(super) updated: Option<i64>,
    pub(super) history: Vec<(i64, String)>,
}

pub(super) fn merge_codex_meta(target: &mut CodexSessionMeta, incoming: CodexSessionMeta) {
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

pub(super) fn codex_configured_sqlite_home(codex_home: &Path) -> Option<PathBuf> {
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

pub(super) fn codex_database_dirs(codex_home: &Path) -> Vec<PathBuf> {
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

pub(super) fn codex_database_paths_all(
    codex_home: &Path,
    prefix: &str,
) -> anyhow::Result<Vec<PathBuf>> {
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

pub(super) fn sqlite_optional_text(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("coalesce(\"{}\", '')", name.replace('"', "\"\""))
    } else {
        "''".to_string()
    }
}

pub(super) fn sqlite_optional_integer(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        "null".to_string()
    }
}

pub(super) fn sqlite_optional_epoch(columns: &[String], name: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("cast(\"{}\" as integer)", name.replace('"', "\"\""))
    } else {
        "null".to_string()
    }
}

pub(super) fn codex_state_metadata_rows(db_path: &Path) -> anyhow::Result<Vec<CodexSessionMeta>> {
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

pub(super) fn codex_catalog_database_paths_all(codex_home: &Path) -> anyhow::Result<Vec<PathBuf>> {
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

pub(super) fn codex_catalog_metadata_rows(db_path: &Path) -> anyhow::Result<Vec<CodexSessionMeta>> {
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
pub(super) enum CodexTimestamp {
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
pub(super) struct CodexSessionIndexEntry {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) thread_name: Option<String>,
    #[serde(default)]
    pub(super) updated_at: Option<CodexTimestamp>,
}

#[derive(serde::Deserialize)]
pub(super) struct CodexHistoryEntry {
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) ts: Option<CodexTimestamp>,
}

pub(super) fn codex_metadata_result(
    root: &Path,
) -> anyhow::Result<HashMap<String, CodexSessionMeta>> {
    Ok((*codex_metadata_snapshot(root)?).clone())
}

pub(super) fn read_codex_metadata(
    root: &Path,
) -> anyhow::Result<(HashMap<String, CodexSessionMeta>, bool)> {
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut complete = true;
    let mut out: HashMap<String, CodexSessionMeta> = HashMap::new();

    for db_path in codex_database_paths_all(codex_home, "state_")? {
        let Ok(rows) = codex_state_metadata_rows(&db_path) else {
            complete = false;
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
            complete = false;
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
        let Ok(file) = fs::File::open(&path) else {
            complete &= !path.exists();
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<CodexSessionIndexEntry>(&line) else {
                continue;
            };
            let Some(id) = value.id.as_deref().and_then(extract_uuid) else {
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
            complete &= !codex_home.join(name).exists();
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<CodexHistoryEntry>(&line) else {
                continue;
            };
            let Some(id) = value.session_id.as_deref().and_then(extract_uuid) else {
                continue;
            };
            let text = value.text.as_deref().unwrap_or_default().trim();
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
    Ok((out, complete))
}

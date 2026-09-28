use super::*;

pub(super) fn read_antigravity_index(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn antigravity_summary_index_result(
    root: &Path,
) -> anyhow::Result<HashMap<String, (String, String)>> {
    let Some(bytes) = read_antigravity_index(&root.join("agyhub_summaries_proto.pb"))? else {
        return Ok(HashMap::new());
    };
    let mut out = HashMap::new();

    for (entry, id) in protobuf_map_entries_result(&bytes)? {
        let strings = printable_strings(entry);
        let title = strings
            .iter()
            .map(|s| clean_proto_text(s))
            .find(|s| {
                !s.is_empty()
                    && extract_uuid(s).is_none()
                    && !s.starts_with("file://")
                    && !s.starts_with("https://")
                    && s != "master"
            })
            .unwrap_or_else(|| "Antigravity 会话".to_string());

        let cwd = strings
            .iter()
            .find(|s| s.contains("file:///"))
            .map(|s| {
                let start = s.find("file:///").unwrap_or(0);
                clean_file_uri(&s[start..])
            })
            .unwrap_or_default();

        out.insert(id, (truncate(&title, MAX_TITLE_CHARS), cwd));
    }

    Ok(out)
}

pub(super) fn antigravity_summary_index(root: &Path) -> HashMap<String, (String, String)> {
    antigravity_summary_index_result(root).unwrap_or_default()
}

pub(super) fn antigravity_strings_to_parsed(bytes: &[u8]) -> Parsed {
    let strings = printable_strings(bytes);
    let mut p = Parsed {
        cwd: String::new(),
        title: String::new(),
        created: None,
        updated: None,
        blocks: Vec::new(),
    };
    for s in strings {
        let text = clean_proto_text(&s);
        if p.cwd.is_empty() && text.contains("file:///") {
            let start = text.find("file:///").unwrap_or(0);
            p.cwd = clean_file_uri(&text[start..]);
            continue;
        }
        if !is_readable_proto_text(&text) {
            continue;
        }
        push_block(&mut p.blocks, "artifact", text);
        if p.blocks.len() >= 24 {
            break;
        }
    }
    p.title = title_from_any(&p.blocks);
    p
}

pub(super) fn antigravity_ide_state_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("Antigravity IDE")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    })
}

pub(super) fn antigravity_ide_index_result(
) -> anyhow::Result<HashMap<String, AntigravityIdeIndexEntry>> {
    let Some(db) = antigravity_ide_state_db() else {
        return Ok(HashMap::new());
    };
    match fs::metadata(&db) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error.into()),
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let trajectory_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let mut out = HashMap::new();
    if let Some(value) = trajectory_value {
        let bytes = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 轨迹索引失败: {error}"))?;
        for (payload, id) in protobuf_map_entries_result(&bytes)? {
            let strings = printable_strings(payload);
            let mut entry = strings
                .iter()
                .find_map(|s| antigravity_ide_entry_from_base64(s))
                .unwrap_or_default();
            if entry.title.is_empty() {
                entry.title = strings
                    .iter()
                    .map(|s| clean_proto_text(s))
                    .find(|s| is_antigravity_ide_title_candidate(s))
                    .unwrap_or_default();
            }
            if entry.cwd.is_empty() {
                entry.cwd = strings
                    .iter()
                    .find_map(|s| clean_file_uri_at(s))
                    .unwrap_or_default();
            }
            out.insert(id, entry);
        }
    }

    let artifact_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(value) = artifact_value {
        let bytes = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 产物索引失败: {error}"))?;
        for (id, title) in antigravity_ide_artifact_titles(&bytes)? {
            let entry = out.entry(id).or_default();
            if entry.title.is_empty() {
                entry.title = title;
            }
        }
    }
    Ok(out)
}

pub(super) fn antigravity_ide_artifact_title(payload: &[u8]) -> Option<String> {
    let json = if payload.first().is_some_and(|byte| *byte == b'{') {
        payload
    } else {
        protobuf_fields(payload)
            .ok()?
            .into_iter()
            .filter_map(|field| field.value)
            .find(|value| value.first().is_some_and(|byte| *byte == b'{'))?
    };
    let value = serde_json::from_slice::<serde_json::Value>(json).ok()?;
    let encoded = value.get("artifactMetadata")?.as_str()?;
    let bytes = general_purpose::STANDARD.decode(encoded).ok()?;
    printable_strings(&bytes)
        .into_iter()
        .map(|value| clean_proto_text(&value))
        .find(|value| {
            (is_readable_proto_text(value) || is_antigravity_ide_title_candidate(value))
                && extract_uuid(value).is_none()
        })
        .map(|value| truncate(&value, MAX_TITLE_CHARS))
}

pub(super) fn antigravity_ide_artifact_titles(
    bytes: &[u8],
) -> anyhow::Result<HashMap<String, String>> {
    let mut candidates: HashMap<String, (u8, String)> = HashMap::new();
    for (path, payload) in protobuf_string_map_entries_result(bytes)? {
        let Some(id) = extract_uuid(&path) else {
            continue;
        };
        let title = antigravity_ide_artifact_title(payload).unwrap_or_default();
        let priority = if path.ends_with("task.md") {
            3
        } else if path.ends_with("implementation_plan.md") {
            2
        } else {
            1
        };
        let replace = match candidates.get(&id) {
            None => true,
            Some((current, current_title)) => {
                !title.is_empty() && (current_title.is_empty() || priority > *current)
            }
        };
        if replace {
            candidates.insert(id, (priority, title));
        } else {
            candidates.entry(id).or_insert((0, String::new()));
        }
    }
    Ok(candidates
        .into_iter()
        .map(|(id, (_, title))| (id, title))
        .collect())
}

pub(super) fn antigravity_ide_artifact_blocks(id: &str) -> Vec<(String, String)> {
    let Some(db) = antigravity_ide_state_db() else {
        return Vec::new();
    };
    let Ok(conn) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Vec::new();
    };
    let Ok(value) = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .optional()
    else {
        return Vec::new();
    };
    let Some(value) = value else {
        return Vec::new();
    };
    let Ok(bytes) = general_purpose::STANDARD.decode(value) else {
        return Vec::new();
    };
    let Ok(entries) = protobuf_string_map_entries_result(&bytes) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|(path, _)| extract_uuid(path).as_deref() == Some(id))
        .filter_map(|(path, payload)| {
            let title = antigravity_ide_artifact_title(payload)?;
            let label = if path.ends_with("task.md") {
                "任务"
            } else if path.ends_with("implementation_plan.md") {
                "实施计划"
            } else if path.ends_with("walkthrough.md") {
                "完成说明"
            } else {
                "产物"
            };
            Some(("artifact".to_string(), format!("{label}：{title}")))
        })
        .collect()
}

pub(super) fn antigravity_ide_index() -> HashMap<String, AntigravityIdeIndexEntry> {
    antigravity_ide_index_result().unwrap_or_default()
}

pub(super) fn looks_base64_blob(value: &str) -> bool {
    value.len() > 120
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
}

pub(super) fn antigravity_ide_entry_from_base64(value: &str) -> Option<AntigravityIdeIndexEntry> {
    if !looks_base64_blob(value) {
        return None;
    }
    let bytes = general_purpose::STANDARD.decode(value).ok()?;
    let strings = printable_strings(&bytes);
    let title = strings
        .iter()
        .map(|s| clean_proto_text(s))
        .find(|s| is_antigravity_ide_title_candidate(s))
        .unwrap_or_default();
    let cwd = strings
        .iter()
        .find_map(|s| clean_file_uri_at(s))
        .unwrap_or_default();
    if title.is_empty() && cwd.is_empty() {
        return None;
    }
    Some(AntigravityIdeIndexEntry { title, cwd })
}

pub(super) fn is_antigravity_ide_title_candidate(value: &str) -> bool {
    let s = value.trim();
    !s.is_empty()
        && s.chars().count() <= MAX_TITLE_CHARS
        && extract_uuid(s).is_none()
        && !s.starts_with("file://")
        && !s.starts_with("http")
        && !looks_base64_blob(s)
        && !s.contains('<')
        && !s.contains('>')
        && !s.contains("\\")
}

pub(super) fn read_artifact_meta(path: &Path) -> Option<AntigravityArtifactMeta> {
    serde_json::from_reader(BufReader::new(fs::File::open(path).ok()?)).ok()
}

pub(super) fn antigravity_ide_conversation_path(root: &Path, id: &str) -> PathBuf {
    let db = root.join("conversations").join(format!("{id}.db"));
    if db.exists() {
        db
    } else {
        root.join("conversations").join(format!("{id}.pb"))
    }
}

pub(super) fn antigravity_ide_sidecar_size(path: &Path) -> u64 {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .iter()
    .map(|candidate| fs::metadata(candidate).map(|meta| meta.len()).unwrap_or(0))
    .sum()
}

pub(super) fn antigravity_ide_db_stats_result(path: &Path) -> anyhow::Result<(String, usize)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let tables = sqlite_table_names(&conn)?;
    let cwd = if tables
        .iter()
        .any(|table| table == "trajectory_metadata_blob")
    {
        conn.query_row(
            "select data from trajectory_metadata_blob limit 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?
        .and_then(|bytes| {
            printable_strings(&bytes)
                .into_iter()
                .find_map(|s| clean_file_uri_at(&s))
        })
        .unwrap_or_default()
    } else {
        String::new()
    };
    let count = if tables.iter().any(|table| table == "steps") {
        conn.query_row("select count(*) from steps", [], |row| row.get::<_, i64>(0))?
            .max(0) as usize
    } else {
        0
    };
    Ok((cwd, count))
}

pub(super) fn antigravity_ide_db_stats(path: &Path) -> (String, usize) {
    antigravity_ide_db_stats_result(path).unwrap_or_default()
}

pub(super) fn antigravity_ide_db_blocks(path: &Path) -> Vec<String> {
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("select step_payload from steps order by idx") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0)) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut blocks = Vec::new();
    for row in rows.flatten() {
        for raw in printable_strings(&row) {
            let text = clean_proto_text(&raw);
            if !is_readable_proto_text(&text)
                || text.chars().count() > 8_000
                || text.contains("# Conversation History")
                || !seen.insert(text.clone())
            {
                continue;
            }
            blocks.push(text);
            if blocks.len() >= 24 {
                return blocks;
            }
        }
    }
    blocks
}

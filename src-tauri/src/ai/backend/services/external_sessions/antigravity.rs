#[derive(Clone, Copy)]
struct ProtoField<'a> {
    number: u64,
    wire_type: u8,
    raw: &'a [u8],
    value: Option<&'a [u8]>,
}

fn read_proto_varint(bytes: &[u8], index: &mut usize) -> anyhow::Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes
            .get(*index)
            .ok_or_else(|| anyhow::anyhow!("截断的 protobuf varint"))?;
        *index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(anyhow::anyhow!("protobuf varint 过长"))
}

fn protobuf_fields(bytes: &[u8]) -> anyhow::Result<Vec<ProtoField<'_>>> {
    let mut fields = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let start = index;
        let key = read_proto_varint(bytes, &mut index)?;
        let number = key >> 3;
        let wire_type = (key & 0x07) as u8;
        let value = match wire_type {
            0 => {
                read_proto_varint(bytes, &mut index)?;
                None
            }
            1 => {
                index = index
                    .checked_add(8)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf fixed64"))?;
                None
            }
            2 => {
                let len = usize::try_from(read_proto_varint(bytes, &mut index)?)
                    .map_err(|_| anyhow::anyhow!("protobuf 字段长度过大"))?;
                let end = index
                    .checked_add(len)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf bytes 字段"))?;
                let value = &bytes[index..end];
                index = end;
                Some(value)
            }
            5 => {
                index = index
                    .checked_add(4)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("截断的 protobuf fixed32"))?;
                None
            }
            _ => return Err(anyhow::anyhow!("不支持的 protobuf wire type: {wire_type}")),
        };
        fields.push(ProtoField {
            number,
            wire_type,
            raw: &bytes[start..index],
            value,
        });
    }
    Ok(fields)
}

fn protobuf_map_entry_id(entry: &[u8]) -> Option<String> {
    protobuf_fields(entry)
        .ok()?
        .into_iter()
        .find(|field| field.number == 1 && field.wire_type == 2)
        .and_then(|field| field.value)
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(extract_uuid)
}

fn protobuf_map_entries_result(bytes: &[u8]) -> anyhow::Result<Vec<(&[u8], String)>> {
    Ok(protobuf_fields(bytes)?
        .into_iter()
        .filter(|field| field.number == 1 && field.wire_type == 2)
        .filter_map(|field| {
            let value = field.value?;
            Some((value, protobuf_map_entry_id(value)?))
        })
        .collect())
}

fn protobuf_string_map_entries_result(bytes: &[u8]) -> anyhow::Result<Vec<(String, &[u8])>> {
    let mut out = Vec::new();
    for field in protobuf_fields(bytes)? {
        if field.number != 1 || field.wire_type != 2 {
            continue;
        }
        let Some(entry) = field.value else {
            continue;
        };
        let fields = protobuf_fields(entry)?;
        let key = fields
            .iter()
            .find(|field| field.number == 1 && field.wire_type == 2)
            .and_then(|field| field.value)
            .and_then(|value| std::str::from_utf8(value).ok())
            .map(str::to_string);
        let value = fields
            .iter()
            .find(|field| field.number == 2 && field.wire_type == 2)
            .and_then(|field| field.value);
        if let (Some(key), Some(value)) = (key, value) {
            out.push((key, value));
        }
    }
    Ok(out)
}

#[cfg(test)]
fn protobuf_map_entries(bytes: &[u8]) -> Vec<(&[u8], String)> {
    protobuf_map_entries_result(bytes).unwrap_or_default()
}

fn remove_protobuf_map_entries(
    bytes: &[u8],
    ids: &HashSet<String>,
) -> anyhow::Result<(Vec<u8>, usize)> {
    let fields = protobuf_fields(bytes)?;
    let mut output = Vec::with_capacity(bytes.len());
    let mut removed = 0usize;
    for field in fields {
        let remove = field.number == 1
            && field.wire_type == 2
            && field
                .value
                .and_then(protobuf_map_entry_id)
                .is_some_and(|id| ids.contains(&id));
        if remove {
            removed += 1;
        } else {
            output.extend_from_slice(field.raw);
        }
    }
    Ok((output, removed))
}

fn remove_protobuf_string_map_entries(
    bytes: &[u8],
    ids: &HashSet<String>,
) -> anyhow::Result<(Vec<u8>, usize)> {
    if ids.is_empty() {
        return Ok((bytes.to_vec(), 0));
    }
    let fields = protobuf_fields(bytes)?;
    let mut output = Vec::with_capacity(bytes.len());
    let mut removed = 0usize;
    for field in fields {
        let remove = field.number == 1
            && field.wire_type == 2
            && field.value.is_some_and(|entry| {
                protobuf_fields(entry)
                    .ok()
                    .and_then(|fields| {
                        fields
                            .into_iter()
                            .find(|field| field.number == 1 && field.wire_type == 2)
                    })
                    .and_then(|field| field.value)
                    .and_then(|value| std::str::from_utf8(value).ok())
                    .and_then(extract_uuid)
                    .is_some_and(|id| ids.contains(&id))
            });
        if remove {
            removed += 1;
        } else {
            output.extend_from_slice(field.raw);
        }
    }
    Ok((output, removed))
}

fn read_antigravity_index(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn antigravity_summary_index_result(
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

fn antigravity_summary_index(root: &Path) -> HashMap<String, (String, String)> {
    antigravity_summary_index_result(root).unwrap_or_default()
}

fn antigravity_strings_to_parsed(bytes: &[u8]) -> Parsed {
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

fn antigravity_ide_state_db() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("Antigravity IDE")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    })
}

fn antigravity_ide_index_result() -> anyhow::Result<HashMap<String, AntigravityIdeIndexEntry>> {
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

fn antigravity_ide_artifact_title(payload: &[u8]) -> Option<String> {
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

fn antigravity_ide_artifact_titles(bytes: &[u8]) -> anyhow::Result<HashMap<String, String>> {
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

fn antigravity_ide_artifact_blocks(id: &str) -> Vec<(String, String)> {
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

fn antigravity_ide_index() -> HashMap<String, AntigravityIdeIndexEntry> {
    antigravity_ide_index_result().unwrap_or_default()
}

fn looks_base64_blob(value: &str) -> bool {
    value.len() > 120
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
}

fn antigravity_ide_entry_from_base64(value: &str) -> Option<AntigravityIdeIndexEntry> {
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

fn is_antigravity_ide_title_candidate(value: &str) -> bool {
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

fn read_artifact_meta(path: &Path) -> Option<AntigravityArtifactMeta> {
    serde_json::from_reader(BufReader::new(fs::File::open(path).ok()?)).ok()
}

fn antigravity_ide_conversation_path(root: &Path, id: &str) -> PathBuf {
    let db = root.join("conversations").join(format!("{id}.db"));
    if db.exists() {
        db
    } else {
        root.join("conversations").join(format!("{id}.pb"))
    }
}

fn antigravity_ide_sidecar_size(path: &Path) -> u64 {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .iter()
    .map(|candidate| fs::metadata(candidate).map(|meta| meta.len()).unwrap_or(0))
    .sum()
}

fn antigravity_ide_db_stats_result(path: &Path) -> anyhow::Result<(String, usize)> {
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

fn antigravity_ide_db_stats(path: &Path) -> (String, usize) {
    antigravity_ide_db_stats_result(path).unwrap_or_default()
}

fn antigravity_ide_db_blocks(path: &Path) -> Vec<String> {
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

fn antigravity_ide_summary(
    root: &Path,
    id: &str,
    dir: &Path,
    index: &HashMap<String, AntigravityIdeIndexEntry>,
) -> SessionSummary {
    let task = dir.join("task.md");
    let plan = dir.join("implementation_plan.md");
    let walkthrough = dir.join("walkthrough.md");
    let transcript = dir
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    let conv = antigravity_ide_conversation_path(root, id);

    let task_meta = read_artifact_meta(&dir.join("task.md.metadata.json"));
    let plan_meta = read_artifact_meta(&dir.join("implementation_plan.md.metadata.json"));
    let walkthrough_meta = read_artifact_meta(&dir.join("walkthrough.md.metadata.json"));

    let title = index
        .get(id)
        .map(|e| e.title.trim())
        .filter(|s| !s.is_empty())
        .map(|s| truncate(s, MAX_TITLE_CHARS))
        .or_else(|| {
            task_meta
                .as_ref()
                .map(|m| m.summary.trim())
                .filter(|s| !s.is_empty())
                .map(|s| truncate(s, MAX_TITLE_CHARS))
        })
        .or_else(|| first_markdown_heading(&task))
        .or_else(|| first_markdown_heading(&plan))
        .or_else(|| first_markdown_heading(&walkthrough))
        .unwrap_or_else(|| "Antigravity IDE 会话".to_string());

    let cwd = index
        .get(id)
        .map(|e| e.cwd.trim())
        .filter(|s| !s.is_empty())
        .map(|s| normalize_workspace_path(s.to_string()))
        .or_else(|| antigravity_ide_workspace_from_dir(dir))
        .or_else(|| {
            (conv.extension().and_then(|ext| ext.to_str()) == Some("db"))
                .then(|| antigravity_ide_db_stats(&conv).0)
                .filter(|cwd| !cwd.is_empty())
        })
        .unwrap_or_else(|| "Antigravity IDE".to_string());

    let file_size = [
        task.as_path(),
        plan.as_path(),
        walkthrough.as_path(),
        transcript.as_path(),
    ]
    .iter()
    .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
    .sum::<u64>()
        + if conv.extension().and_then(|ext| ext.to_str()) == Some("db") {
            antigravity_ide_sidecar_size(&conv)
        } else {
            fs::metadata(&conv).map(|meta| meta.len()).unwrap_or(0)
        };
    let modified_at = [
        task_meta
            .as_ref()
            .and_then(|m| m.updated_at.as_deref())
            .and_then(iso_secs),
        plan_meta
            .as_ref()
            .and_then(|m| m.updated_at.as_deref())
            .and_then(iso_secs),
        walkthrough_meta
            .as_ref()
            .and_then(|m| m.updated_at.as_deref())
            .and_then(iso_secs),
        metadata_secs(dir, false),
        metadata_secs(&conv, false),
    ]
    .into_iter()
    .flatten()
    .max();

    SessionSummary {
        session_id: format!("brain/{id}"),
        title,
        session_type: "antigravity-ide".to_string(),
        workspace_directory: cwd.clone(),
        workspace_hash: format!("antigravity-ide:{cwd}"),
        message_count: antigravity_ide_message_count(dir, &conv),
        file_size,
        created_at: metadata_secs(dir, true).or_else(|| metadata_secs(&conv, true)),
        modified_at,
        source: "antigravity-ide".to_string(),
    }
}

fn first_markdown_heading(path: &Path) -> Option<String> {
    BufReader::new(fs::File::open(path).ok()?)
        .lines()
        .map_while(Result::ok)
        .find_map(|line| {
            let text = line.trim().trim_start_matches('#').trim();
            (!text.is_empty()).then(|| truncate(text, MAX_TITLE_CHARS))
        })
}

fn antigravity_ide_workspace_from_dir(dir: &Path) -> Option<String> {
    for name in ["task.md", "implementation_plan.md", "walkthrough.md"] {
        let Ok(file) = fs::File::open(dir.join(name)) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(cwd) = clean_file_uri_at(&line) {
                return Some(cwd);
            }
        }
    }
    None
}

fn antigravity_ide_message_count(dir: &Path, conv: &Path) -> usize {
    let mut count = 0;
    for name in ["task.md", "implementation_plan.md", "walkthrough.md"] {
        if fs::metadata(dir.join(name)).map(|m| m.len()).unwrap_or(0) > 0 {
            count += 1;
        }
    }
    let transcript = dir
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");
    if fs::metadata(&transcript).map(|m| m.len()).unwrap_or(0) > 0 {
        count += fs::File::open(&transcript)
            .map(|file| BufReader::new(file).lines().map_while(Result::ok).count())
            .unwrap_or(1);
    }
    if conv.extension().and_then(|ext| ext.to_str()) == Some("db") {
        count += antigravity_ide_db_stats(conv).1;
    } else if fs::metadata(conv).map(|m| m.len()).unwrap_or(0) > 0 {
        count += 1;
    }
    count
}

fn collect_antigravity_ide_source(d: &SourceDef, root: &Path) -> Vec<SessionSummary> {
    let index = antigravity_ide_index();
    let brain = root.join("brain");
    let mut ids = HashSet::new();
    let mut out = Vec::new();

    if let Ok(entries) = fs::read_dir(&brain) {
        for e in entries.flatten() {
            let dir = e.path();
            if !dir.is_dir() {
                continue;
            }
            let Some(id) = e.file_name().to_str().and_then(extract_uuid) else {
                continue;
            };
            if ids.insert(id.clone()) {
                out.push(antigravity_ide_summary(root, &id, &dir, &index));
            }
        }
    }

    let conv_dir = root.join("conversations");
    if let Ok(entries) = fs::read_dir(&conv_dir) {
        for e in entries.flatten() {
            let path = e.path();
            let extension = path.extension().and_then(|s| s.to_str());
            if !matches!(extension, Some("pb") | Some("db")) {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(extract_uuid)
            else {
                continue;
            };
            if !ids.insert(id.clone()) {
                continue;
            }
            let entry = index.get(&id).cloned().unwrap_or_default();
            let (db_cwd, db_count) = if extension == Some("db") {
                antigravity_ide_db_stats(&path)
            } else {
                (String::new(), 1)
            };
            let cwd = if !entry.cwd.is_empty() {
                normalize_workspace_path(entry.cwd)
            } else if !db_cwd.is_empty() {
                normalize_workspace_path(db_cwd)
            } else {
                "Antigravity IDE".to_string()
            };
            let file_size = if extension == Some("db") {
                antigravity_ide_sidecar_size(&path)
            } else {
                fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            };
            out.push(SessionSummary {
                session_id: format!("conversations/{id}.{}", extension.unwrap_or("pb")),
                title: if entry.title.is_empty() {
                    "Antigravity IDE 会话".to_string()
                } else {
                    truncate(&entry.title, MAX_TITLE_CHARS)
                },
                session_type: d.source.to_string(),
                workspace_directory: cwd.clone(),
                workspace_hash: format!("{}{}", d.prefix, cwd),
                message_count: db_count,
                file_size,
                created_at: metadata_secs(&path, true),
                modified_at: metadata_secs(&path, false),
                source: d.source.to_string(),
            });
        }
    }

    let state_db = antigravity_ide_state_db();
    for (id, entry) in index {
        if ids.contains(&id) {
            continue;
        }
        let cwd = if entry.cwd.trim().is_empty() {
            "Antigravity IDE 历史".to_string()
        } else {
            normalize_workspace_path(entry.cwd)
        };
        out.push(SessionSummary {
            session_id: virtual_session_id("trajectory", &id),
            title: if entry.title.trim().is_empty() {
                "Antigravity IDE 会话".to_string()
            } else {
                truncate(&entry.title, MAX_TITLE_CHARS)
            },
            session_type: d.source.to_string(),
            workspace_directory: cwd.clone(),
            workspace_hash: format!("{}{}", d.prefix, cwd),
            message_count: 0,
            file_size: 0,
            created_at: state_db
                .as_deref()
                .and_then(|path| metadata_secs(path, true)),
            modified_at: state_db
                .as_deref()
                .and_then(|path| metadata_secs(path, false)),
            source: d.source.to_string(),
        });
    }

    out
}

fn load_antigravity_summary_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let raw_id = virtual_session_key(session_id, "summary")
        .ok_or_else(|| anyhow::anyhow!("无法识别 Antigravity 摘要会话"))?;
    let id = extract_uuid(raw_id).ok_or_else(|| anyhow::anyhow!("Antigravity 会话 ID 无效"))?;
    let (title, indexed_cwd) = antigravity_summary_index_result(root)?
        .remove(&id)
        .ok_or_else(|| anyhow::anyhow!("Antigravity 摘要索引已不存在，请刷新列表"))?;
    let cwd = if indexed_cwd.trim().is_empty() {
        hash.strip_prefix(d.prefix)
            .unwrap_or("Antigravity 历史")
            .to_string()
    } else {
        indexed_cwd
    };
    Ok(IdeSession {
        session_id: session_id.to_string(),
        title,
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history: vec![history_item(
            "system",
            "Antigravity 会话正文已不在 conversations 目录；当前仅恢复到摘要索引中的标题和工作区信息。".to_string(),
            0,
        )],
        conversation_summary: None,
    })
}

fn load_antigravity_ide_index_session(
    d: &SourceDef,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let raw_id = virtual_session_key(session_id, "trajectory")
        .ok_or_else(|| anyhow::anyhow!("无法识别 Antigravity IDE 索引会话"))?;
    let id = extract_uuid(raw_id).ok_or_else(|| anyhow::anyhow!("Antigravity IDE 会话 ID 无效"))?;
    let entry = antigravity_ide_index_result()?
        .remove(&id)
        .ok_or_else(|| anyhow::anyhow!("Antigravity IDE 索引已不存在，请刷新列表"))?;
    let cwd = if entry.cwd.trim().is_empty() {
        hash.strip_prefix(d.prefix)
            .unwrap_or("Antigravity IDE 历史")
            .to_string()
    } else {
        normalize_workspace_path(entry.cwd)
    };
    let mut history = vec![history_item(
        "system",
        "Antigravity IDE 的 brain/conversations 正文已不在磁盘；当前由 trajectorySummaries 与 artifactReview 恢复只读元数据。".to_string(),
        0,
    )];
    for (role, text) in antigravity_ide_artifact_blocks(&id) {
        history.push(history_item(&role, text, history.len()));
    }
    Ok(IdeSession {
        session_id: session_id.to_string(),
        title: if entry.title.trim().is_empty() {
            "Antigravity IDE 会话".to_string()
        } else {
            truncate(&entry.title, MAX_TITLE_CHARS)
        },
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history,
        conversation_summary: None,
    })
}

fn load_antigravity_ide_session(
    d: &SourceDef,
    root: &Path,
    path: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let mut history = Vec::new();
    let mut title = "Antigravity IDE 会话".to_string();
    let mut cwd = hash.strip_prefix(d.prefix).unwrap_or_default().to_string();

    if session_id.starts_with("brain/") {
        let dir = path;
        if let Some(t) = first_markdown_heading(&dir.join("task.md"))
            .or_else(|| first_markdown_heading(&dir.join("implementation_plan.md")))
            .or_else(|| first_markdown_heading(&dir.join("walkthrough.md")))
        {
            title = t;
        }
        if cwd.is_empty() {
            cwd = antigravity_ide_workspace_from_dir(dir).unwrap_or_default();
        }
        if !cwd.is_empty() {
            history.push(history_item("system", format!("工作目录：{}", cwd), 0));
        }
        push_antigravity_ide_file(&mut history, dir, "task.md", "task.md");
        push_antigravity_ide_file(
            &mut history,
            dir,
            "implementation_plan.md",
            "implementation_plan.md",
        );
        push_antigravity_ide_file(&mut history, dir, "walkthrough.md", "walkthrough.md");
        push_antigravity_ide_file(
            &mut history,
            dir,
            ".system_generated/logs/transcript.jsonl",
            "transcript.jsonl",
        );

        if let Some(id) = session_id.strip_prefix("brain/") {
            let conv = antigravity_ide_conversation_path(root, id);
            append_antigravity_ide_conversation(&mut history, &conv);
        }
    } else {
        if path.extension().and_then(|ext| ext.to_str()) == Some("db") {
            return load_antigravity_ide_db_session(d, path, session_id, hash);
        }
        let bytes = read_binary_file(path).ok_or_else(|| anyhow::anyhow!("无法读取会话文件"))?;
        let parsed = antigravity_strings_to_parsed(&bytes);
        title = if parsed.title == "未命名会话" {
            title
        } else {
            parsed.title
        };
        if cwd.is_empty() {
            cwd = parsed.cwd;
        }
        for (role, text) in parsed.blocks {
            let idx = history.len();
            history.push(history_item(&role, text, idx));
        }
        if history.is_empty() {
            history.push(history_item(
                "assistant",
                "这个 .pb 文件没有解析出可安全展示的文本内容。".to_string(),
                0,
            ));
        }
    }

    Ok(IdeSession {
        session_id: session_id.to_string(),
        title,
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history,
        conversation_summary: None,
    })
}

fn append_antigravity_ide_conversation(history: &mut Vec<HistoryItem>, path: &Path) {
    if path.extension().and_then(|ext| ext.to_str()) == Some("db") {
        let blocks = antigravity_ide_db_blocks(path);
        for text in blocks {
            let idx = history.len();
            history.push(history_item("artifact", text, idx));
        }
        return;
    }
    if let Some(bytes) = read_binary_file(path) {
        let parsed = antigravity_strings_to_parsed(&bytes);
        let readable = parsed
            .blocks
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        if !readable.trim().is_empty() {
            let idx = history.len();
            history.push(history_item(
                "artifact",
                format!("## conversation.pb\n\n{readable}"),
                idx,
            ));
        }
    }
}

fn load_antigravity_ide_db_session(
    d: &SourceDef,
    path: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let (db_cwd, step_count) = antigravity_ide_db_stats(path);
    let id = Path::new(session_id)
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(extract_uuid);
    let title = id
        .and_then(|id| {
            antigravity_ide_index()
                .get(&id)
                .map(|entry| entry.title.clone())
        })
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Antigravity IDE 会话".to_string());
    let cwd = if hash.strip_prefix(d.prefix).unwrap_or_default().is_empty() {
        db_cwd
    } else {
        hash.strip_prefix(d.prefix).unwrap_or_default().to_string()
    };
    let mut history = Vec::new();
    if !cwd.is_empty() {
        history.push(history_item("system", format!("工作目录：{cwd}"), 0));
    }
    for text in antigravity_ide_db_blocks(path) {
        let idx = history.len();
        history.push(history_item("artifact", text, idx));
    }
    if history.is_empty() {
        history.push(history_item(
            "assistant",
            format!("Antigravity 会话数据库，共 {step_count} 个步骤。"),
            0,
        ));
    }
    Ok(IdeSession {
        session_id: session_id.to_string(),
        title,
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history,
        conversation_summary: None,
    })
}

fn push_antigravity_ide_file(history: &mut Vec<HistoryItem>, dir: &Path, rel: &str, label: &str) {
    let path = rel
        .split('/')
        .fold(dir.to_path_buf(), |p, part| p.join(part));
    let Some(content) = read_text_file(&path) else {
        return;
    };
    if content.trim().is_empty() {
        return;
    }
    let idx = history.len();
    history.push(history_item(
        "artifact",
        format!("## {label}\n\n{content}"),
        idx,
    ));
}

fn remove_antigravity_named_artifacts(root: &Path, id: &str, errors: &mut Vec<String>) {
    let canonical_root = match root.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!("校验 {} 失败: {error}", root.display()));
            return;
        }
    };
    remove_antigravity_named_artifacts_in(root, &canonical_root, id, errors);
}

fn remove_antigravity_named_artifacts_in(
    dir: &Path,
    canonical_root: &Path,
    id: &str,
    errors: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("扫描 {} 失败: {error}", dir.display()));
            return;
        }
    };
    let mut entries = match entries
        .map(|entry| entry.map_err(|error| format!("读取 {} 目录项失败: {error}", dir.display())))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    entries.sort_by_key(|entry| {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == format!("{id}.db") || name == format!("{id}.pb") {
            0
        } else if name == format!("{id}.db-wal") || name == format!("{id}.db-shm") {
            2
        } else {
            1
        }
    });
    let mut main_db_failed = false;
    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("读取 {} 类型失败: {error}", path.display()));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let matches = name == id
            || name.starts_with(&format!("{id}."))
            || name.starts_with(&format!("{id}-"));
        if matches {
            if main_db_failed && (name == format!("{id}.db-wal") || name == format!("{id}.db-shm"))
            {
                errors.push(format!(
                    "主数据库删除失败，已保留 SQLite 伴生文件: {}",
                    path.display()
                ));
                continue;
            }
            if file_type.is_symlink() {
                errors.push(format!(
                    "拒绝删除符号链接形式的会话数据: {}",
                    path.display()
                ));
                continue;
            }
            let canonical_path = match path.canonicalize() {
                Ok(path) if path.starts_with(canonical_root) && path != canonical_root => path,
                Ok(_) => {
                    errors.push(format!(
                        "拒绝删除 Antigravity 数据目录之外的路径: {}",
                        path.display()
                    ));
                    continue;
                }
                Err(error) => {
                    errors.push(format!("校验 {} 失败: {error}", path.display()));
                    continue;
                }
            };
            let result = if file_type.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(error) = result {
                if name == format!("{id}.db") {
                    main_db_failed = true;
                }
                errors.push(format!("删除 {} 失败: {error}", canonical_path.display()));
            }
            continue;
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            let inside_root = path
                .canonicalize()
                .is_ok_and(|path| path.starts_with(canonical_root));
            if inside_root {
                remove_antigravity_named_artifacts_in(&path, canonical_root, id, errors);
            } else {
                errors.push(format!(
                    "拒绝扫描 Antigravity 数据目录之外的路径: {}",
                    path.display()
                ));
            }
        }
    }
}

fn antigravity_named_artifacts_remain(root: &Path, ids: &HashSet<String>) -> anyhow::Result<bool> {
    let canonical_root = root.canonicalize()?;
    antigravity_named_artifacts_remain_in(root, &canonical_root, ids)
}

fn antigravity_named_artifacts_remain_in(
    dir: &Path,
    canonical_root: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if ids.iter().any(|id| {
            name == *id
                || name.starts_with(&format!("{id}."))
                || name.starts_with(&format!("{id}-"))
        }) {
            return Ok(true);
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            let canonical = path.canonicalize()?;
            if !canonical.starts_with(canonical_root) {
                return Err(anyhow::anyhow!(
                    "拒绝扫描 Antigravity 数据目录之外的路径: {}",
                    path.display()
                ));
            }
            if antigravity_named_artifacts_remain_in(&path, canonical_root, ids)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn remove_antigravity_proto_index(path: &Path, ids: &HashSet<String>) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(raw) = read_antigravity_index(path)? else {
        return Ok(());
    };
    let (filtered, removed) = remove_protobuf_map_entries(&raw, ids)?;
    if removed == 0 {
        return Ok(());
    }
    crate::ai::backend::utils::fs::atomic_write_bytes(
        path,
        &filtered,
        "Antigravity 会话索引",
    )
    .map_err(anyhow::Error::msg)?;
    Ok(())
}

fn remove_antigravity_ide_state_entries(ids: &HashSet<String>) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(path) = antigravity_ide_state_db() else {
        return Ok(());
    };
    match fs::metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    remove_antigravity_ide_state_entries_at(&path, ids)
}

fn remove_antigravity_ide_state_entries_at(
    path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let trajectory_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let artifact_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let filtered_trajectory = trajectory_value
        .map(|value| {
            let decoded = general_purpose::STANDARD
                .decode(value)
                .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 轨迹索引失败: {error}"))?;
            remove_protobuf_map_entries(&decoded, ids)
        })
        .transpose()?;
    let filtered_artifact = artifact_value
        .map(|value| {
            let decoded = general_purpose::STANDARD
                .decode(value)
                .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 产物索引失败: {error}"))?;
            remove_protobuf_string_map_entries(&decoded, ids)
        })
        .transpose()?;
    let tx = conn.transaction()?;
    if let Some((filtered, removed)) = filtered_trajectory {
        if removed > 0 {
            tx.execute(
                "update ItemTable set value = ?1 where key = ?2",
                rusqlite::params![
                    general_purpose::STANDARD.encode(filtered),
                    "antigravityUnifiedStateSync.trajectorySummaries"
                ],
            )?;
        }
    }
    if let Some((filtered, removed)) = filtered_artifact {
        if removed > 0 {
            tx.execute(
                "update ItemTable set value = ?1 where key = ?2",
                rusqlite::params![
                    general_purpose::STANDARD.encode(filtered),
                    "antigravityUnifiedStateSync.artifactReview"
                ],
            )?;
        }
    }
    for id in ids {
        tx.execute(
            "delete from ItemTable where key like ?1",
            [format!("%{id}%")],
        )?;
    }
    tx.commit()?;
    let _ = conn.execute_batch("pragma wal_checkpoint(truncate)");
    Ok(())
}

fn antigravity_ide_state_entries_remain(ids: &HashSet<String>) -> anyhow::Result<bool> {
    let Some(path) = antigravity_ide_state_db() else {
        return Ok(false);
    };
    match fs::metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let trajectory_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(value) = trajectory_value {
        let decoded = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 轨迹索引失败: {error}"))?;
        if protobuf_map_entries_result(&decoded)?
            .iter()
            .any(|(_, id)| ids.contains(id))
        {
            return Ok(true);
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
        let decoded = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 产物索引失败: {error}"))?;
        if protobuf_string_map_entries_result(&decoded)?
            .iter()
            .any(|(path, _)| extract_uuid(path).is_some_and(|id| ids.contains(&id)))
        {
            return Ok(true);
        }
    }
    for id in ids {
        let remains = conn.query_row(
            "select exists(select 1 from ItemTable where key like ?1)",
            [format!("%{id}%")],
            |row| row.get::<_, bool>(0),
        )?;
        if remains {
            return Ok(true);
        }
    }
    Ok(false)
}

fn antigravity_id_from_session_id(session_id: &str) -> Option<String> {
    session_id
        .strip_prefix("brain/")
        .and_then(extract_uuid)
        .or_else(|| {
            Path::new(session_id)
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(extract_uuid)
        })
}

fn antigravity_artifact_workspace_result(dir: &Path) -> anyhow::Result<Option<String>> {
    for name in ["task.md", "implementation_plan.md", "walkthrough.md"] {
        let path = dir.join(name);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Some(cwd) = clean_file_uri_at(&line) {
                return Ok(Some(cwd));
            }
        }
    }
    Ok(None)
}

fn antigravity_conversation_db_workspace_result(
    root: &Path,
    id: &str,
) -> anyhow::Result<Option<String>> {
    let path = root.join("conversations").join(format!("{id}.db"));
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Antigravity 会话数据库路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    Ok(Some(antigravity_ide_db_stats_result(&path)?.0).filter(|cwd| !cwd.is_empty()))
}

fn optional_read_dir(path: &Path) -> anyhow::Result<Option<fs::ReadDir>> {
    match fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn antigravity_ids_for_workspace(
    d: &SourceDef,
    root: &Path,
    workspace: &str,
) -> anyhow::Result<HashSet<String>> {
    let index = if matches!(d.layout, Layout::AntigravityIde) {
        antigravity_ide_index_result()?
            .into_iter()
            .collect::<HashMap<_, _>>()
    } else {
        antigravity_summary_index_result(root)?
            .into_iter()
            .map(|(id, (_, cwd))| {
                (
                    id,
                    AntigravityIdeIndexEntry {
                        title: String::new(),
                        cwd,
                    },
                )
            })
            .collect::<HashMap<_, _>>()
    };
    antigravity_ids_for_workspace_with_index(d, root, workspace, &index)
}

fn antigravity_ids_for_workspace_with_index(
    d: &SourceDef,
    root: &Path,
    workspace: &str,
    index: &HashMap<String, AntigravityIdeIndexEntry>,
) -> anyhow::Result<HashSet<String>> {
    let target = normalize_codex_workspace(workspace);
    let mut ids = HashSet::new();
    let default_workspace = if matches!(d.layout, Layout::AntigravityIde) {
        "Antigravity IDE"
    } else {
        "Antigravity"
    };
    for (id, entry) in index {
        let cwd = if entry.cwd.trim().is_empty() {
            default_workspace
        } else {
            &entry.cwd
        };
        if normalize_codex_workspace(cwd) == target {
            ids.insert(id.clone());
        }
    }

    let scan_root = root.join("conversations");
    if let Some(entries) = optional_read_dir(&scan_root)? {
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_db = name.ends_with(".db");
            let is_pb = name.ends_with(".pb");
            let is_db_sidecar = name.ends_with(".db-wal") || name.ends_with(".db-shm");
            if !is_db && !is_pb && !is_db_sidecar {
                continue;
            }
            let Some(id) = extract_uuid(&name) else {
                continue;
            };
            let indexed_cwd = index
                .get(&id)
                .map(|entry| entry.cwd.trim().to_string())
                .filter(|cwd| !cwd.is_empty());
            let db_path = if is_db {
                Some(path)
            } else if is_db_sidecar {
                let candidate = scan_root.join(format!("{id}.db"));
                match fs::metadata(&candidate) {
                    Ok(metadata) if metadata.is_file() => Some(candidate),
                    Ok(_) => {
                        return Err(anyhow::anyhow!(
                            "Antigravity 会话数据库路径不是文件: {}",
                            candidate.display()
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                }
            } else {
                None
            };
            let db_cwd = db_path
                .as_deref()
                .map(antigravity_ide_db_stats_result)
                .transpose()?
                .map(|(cwd, _)| cwd)
                .filter(|cwd| !cwd.is_empty());
            let cwd = indexed_cwd
                .or(db_cwd)
                .unwrap_or_else(|| default_workspace.to_string());
            if normalize_codex_workspace(&cwd) == target {
                ids.insert(id);
            }
        }
    }
    if matches!(d.layout, Layout::AntigravityIde) {
        let brain = root.join("brain");
        if let Some(entries) = optional_read_dir(&brain)? {
            for entry in entries {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if !file_type.is_dir() || file_type.is_symlink() {
                    continue;
                }
                let Some(id) = entry.file_name().to_str().and_then(extract_uuid) else {
                    continue;
                };
                let cwd = index
                    .get(&id)
                    .map(|entry| entry.cwd.trim().to_string())
                    .filter(|cwd| !cwd.is_empty())
                    .or(antigravity_artifact_workspace_result(&entry.path())?)
                    .or(antigravity_conversation_db_workspace_result(root, &id)?)
                    .unwrap_or_else(|| default_workspace.to_string());
                if normalize_codex_workspace(&cwd) == target {
                    ids.insert(id);
                }
            }
        }
    }
    Ok(ids)
}

fn delete_antigravity_ids(d: &SourceDef, root: &Path, ids: &HashSet<String>) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut errors = Vec::new();
    if matches!(d.layout, Layout::AntigravityIde) {
        if let Err(error) = remove_antigravity_ide_state_entries(ids) {
            errors.push(format!("清理 Antigravity IDE 轨迹索引失败: {error}"));
        }
    } else {
        let index_path = root.join("agyhub_summaries_proto.pb");
        if let Err(error) = remove_antigravity_proto_index(&index_path, ids) {
            errors.push(format!("清理 Antigravity 摘要索引失败: {error}"));
        }
    }
    // 状态索引无法更新时保留会话正文，避免产生无法从界面再次定位的孤儿记录。
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(errors.join("；")));
    }
    let metadata_remains = if matches!(d.layout, Layout::AntigravityIde) {
        antigravity_ide_state_entries_remain(ids)?
    } else {
        antigravity_summary_index_result(root)?
            .keys()
            .any(|id| ids.contains(id))
    };
    if metadata_remains {
        return Err(anyhow::anyhow!(
            "Antigravity 会话索引仍有残留，已保留正文文件以便重试"
        ));
    }
    for id in ids {
        remove_antigravity_named_artifacts(root, id, &mut errors);
    }
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(errors.join("；")));
    }
    if antigravity_named_artifacts_remain(root, ids)? {
        return Err(anyhow::anyhow!(
            "Antigravity 会话文件仍有残留，请关闭 Antigravity 后重试"
        ));
    }
    Ok(())
}

fn delete_antigravity_session(d: &SourceDef, root: &Path, session_id: &str) -> anyhow::Result<()> {
    let id = antigravity_id_from_session_id(session_id)
        .ok_or_else(|| anyhow::anyhow!("无法识别 Antigravity 会话 ID"))?;
    delete_antigravity_ids(d, root, &HashSet::from([id]))
}

fn delete_antigravity_workspace(d: &SourceDef, root: &Path, workspace: &str) -> anyhow::Result<()> {
    if normalize_codex_workspace(workspace).is_empty() {
        return Err(anyhow::anyhow!("Antigravity 工作区路径为空，已拒绝删除"));
    }
    let ids = antigravity_ids_for_workspace(d, root, workspace)?;
    delete_antigravity_ids(d, root, &ids)?;
    if !antigravity_ids_for_workspace(d, root, workspace)?.is_empty() {
        return Err(anyhow::anyhow!(
            "Antigravity 工作区仍有残留记录，请关闭 Antigravity 后重试"
        ));
    }
    Ok(())
}

fn parse_antigravity(content: &str) -> Parsed {
    let mut p = Parsed {
        cwd: String::new(),
        title: String::new(),
        created: None,
        updated: None,
        blocks: Vec::new(),
    };
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = v
            .get("created_at")
            .and_then(|x| x.as_str())
            .and_then(iso_secs)
        {
            if p.created.is_none() {
                p.created = Some(ts);
            }
            p.updated = Some(ts);
        }
        let role = if v.get("source").and_then(|x| x.as_str()) == Some("USER_EXPLICIT") {
            "user"
        } else {
            "assistant"
        };
        let raw = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
        let text = match (raw.find("<USER_REQUEST>"), raw.find("</USER_REQUEST>")) {
            (Some(a), Some(b)) if b > a => raw[a + "<USER_REQUEST>".len()..b].to_string(),
            _ => raw.to_string(),
        };
        push_block(&mut p.blocks, role, text);
    }
    p.title = title_from(&p.blocks);
    p
}

#[cfg(test)]
mod antigravity_tests {
    use super::*;

    fn no_test_root() -> Option<PathBuf> {
        None
    }

    fn no_test_summary(_: &Path) -> Option<ParsedSummary> {
        None
    }

    fn test_source(layout: Layout) -> SourceDef {
        SourceDef {
            prefix: "test:",
            source: "test",
            root: no_test_root,
            layout,
            parse: parse_antigravity,
            scan: no_test_summary,
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn varint(mut value: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return out;
            }
        }
    }

    fn map_entry(id: &str, text: &str) -> Vec<u8> {
        let mut payload = vec![0x0a, 36];
        payload.extend_from_slice(id.as_bytes());
        payload.extend_from_slice(&[0x12, text.len() as u8]);
        payload.extend_from_slice(text.as_bytes());
        let mut field = vec![0x0a];
        field.extend(varint(payload.len()));
        field.extend(payload);
        field
    }

    fn bytes_field(number: usize, payload: &[u8]) -> Vec<u8> {
        let mut field = varint((number << 3) | 2);
        field.extend(varint(payload.len()));
        field.extend_from_slice(payload);
        field
    }

    fn string_map_entry(key: &str, value: &[u8]) -> Vec<u8> {
        let mut entry = bytes_field(1, key.as_bytes());
        entry.extend(bytes_field(2, value));
        bytes_field(1, &entry)
    }

    #[test]
    fn removes_only_matching_antigravity_summary_entries() {
        let keep = "11111111-1111-4111-8111-111111111111";
        let remove = "22222222-2222-4222-8222-222222222222";
        let mut bytes = map_entry(keep, "keep");
        bytes.extend(map_entry(remove, "remove"));
        let (filtered, count) =
            remove_protobuf_map_entries(&bytes, &HashSet::from([remove.to_string()])).unwrap();
        assert_eq!(count, 1);
        assert_eq!(protobuf_map_entries(&filtered).len(), 1);
        assert_eq!(protobuf_map_entries(&filtered)[0].1, keep);
    }

    #[test]
    fn rejects_truncated_antigravity_index_instead_of_overwriting_it() {
        let result = remove_protobuf_map_entries(&[0x0a, 0xff], &HashSet::new());
        assert!(result.is_err());
    }

    #[test]
    fn restores_antigravity_artifact_review_ids_and_titles() {
        let titled = "11111111-1111-4111-8111-111111111111";
        let untitled = "22222222-2222-4222-8222-222222222222";
        let metadata = bytes_field(1, "恢复的实施计划".as_bytes());
        let json = serde_json::to_vec(&serde_json::json!({
            "artifactMetadata": general_purpose::STANDARD.encode(metadata)
        }))
        .unwrap();
        let wrapped = bytes_field(1, &json);
        let mut bytes = string_map_entry(&format!("file:///tmp/brain/{titled}/task.md"), &wrapped);
        let empty_json = bytes_field(1, br#"{"comments":[]}"#);
        bytes.extend(string_map_entry(
            &format!("file:///tmp/brain/{untitled}/walkthrough.md"),
            &empty_json,
        ));

        let titles = antigravity_ide_artifact_titles(&bytes).unwrap();
        assert_eq!(titles.len(), 2);
        assert_eq!(
            titles.get(titled).map(String::as_str),
            Some("恢复的实施计划")
        );
        assert_eq!(titles.get(untitled).map(String::as_str), Some(""));
    }

    #[test]
    fn keeps_antigravity_artifacts_when_index_cleanup_fails() {
        let root = test_dir("antigravity-index-failure");
        let conversations = root.join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let id = "22222222-2222-4222-8222-222222222222";
        let conversation = conversations.join(format!("{id}.pb"));
        fs::write(&conversation, b"conversation").unwrap();
        fs::write(root.join("agyhub_summaries_proto.pb"), [0x0a, 0xff]).unwrap();
        let source = SOURCES
            .iter()
            .find(|source| source.source == "antigravity")
            .unwrap();

        assert!(delete_antigravity_ids(source, &root, &HashSet::from([id.to_string()])).is_err());
        assert!(conversation.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_antigravity_named_artifacts_without_touching_root() {
        let root = test_dir("antigravity-artifacts");
        let conversations = root.join("conversations");
        let brain = root.join("brain");
        fs::create_dir_all(&conversations).unwrap();
        fs::create_dir_all(&brain).unwrap();
        let id = "22222222-2222-4222-8222-222222222222";
        for suffix in [".db", ".db-wal", ".db-shm", ".pb"] {
            fs::write(conversations.join(format!("{id}{suffix}")), b"data").unwrap();
        }
        fs::create_dir_all(brain.join(id)).unwrap();
        let keep = conversations.join("keep.db");
        fs::write(&keep, b"keep").unwrap();
        let mut errors = Vec::new();

        remove_antigravity_named_artifacts(&root, id, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(root.exists());
        assert!(keep.exists());
        assert!(!brain.join(id).exists());
        for suffix in [".db", ".db-wal", ".db-shm", ".pb"] {
            assert!(!conversations.join(format!("{id}{suffix}")).exists());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_empty_antigravity_workspace() {
        let root = test_dir("antigravity-empty-workspace");
        let source = SOURCES
            .iter()
            .find(|source| source.source == "antigravity")
            .unwrap();
        assert!(delete_antigravity_workspace(source, &root, "").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removes_antigravity_trajectory_and_notification_state() {
        let dir = test_dir("antigravity-state");
        let db = dir.join("state.vscdb");
        let keep = "11111111-1111-4111-8111-111111111111";
        let remove = "22222222-2222-4222-8222-222222222222";
        let mut bytes = map_entry(keep, "keep");
        bytes.extend(map_entry(remove, "remove"));
        let artifact = string_map_entry(
            &format!("file:///tmp/brain/{remove}/task.md"),
            &bytes_field(1, br#"{}"#),
        );
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("create table ItemTable (key text primary key, value text);")
            .unwrap();
        conn.execute(
            "insert into ItemTable (key, value) values (?1, ?2)",
            rusqlite::params![
                "antigravityUnifiedStateSync.trajectorySummaries",
                general_purpose::STANDARD.encode(bytes)
            ],
        )
        .unwrap();
        conn.execute(
            "insert into ItemTable (key, value) values (?1, ?2)",
            rusqlite::params![
                "antigravityUnifiedStateSync.artifactReview",
                general_purpose::STANDARD.encode(artifact)
            ],
        )
        .unwrap();
        conn.execute(
            "insert into ItemTable (key, value) values (?1, '1')",
            [format!("antigravity.notification.{remove}")],
        )
        .unwrap();
        drop(conn);

        remove_antigravity_ide_state_entries_at(&db, &HashSet::from([remove.to_string()])).unwrap();
        let conn = Connection::open(&db).unwrap();
        let value = conn
            .query_row(
                "select value from ItemTable where key = ?1",
                ["antigravityUnifiedStateSync.trajectorySummaries"],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let decoded = general_purpose::STANDARD.decode(value).unwrap();
        let entries = protobuf_map_entries(&decoded);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, keep);
        let artifact = conn
            .query_row(
                "select value from ItemTable where key = ?1",
                ["antigravityUnifiedStateSync.artifactReview"],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let artifact = general_purpose::STANDARD.decode(artifact).unwrap();
        assert!(protobuf_string_map_entries_result(&artifact)
            .unwrap()
            .is_empty());
        assert_eq!(
            conn.query_row(
                "select count(*) from ItemTable where key like ?1",
                [format!("%{remove}%")],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        drop(conn);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removes_antigravity_notification_without_trajectory_index() {
        let dir = test_dir("antigravity-notification-only");
        let db = dir.join("state.vscdb");
        let remove = "22222222-2222-4222-8222-222222222222";
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("create table ItemTable (key text primary key, value text);")
            .unwrap();
        conn.execute(
            "insert into ItemTable (key, value) values (?1, '1')",
            [format!("antigravity.notification.10{remove}")],
        )
        .unwrap();
        drop(conn);

        remove_antigravity_ide_state_entries_at(&db, &HashSet::from([remove.to_string()])).unwrap();
        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            conn.query_row("select count(*) from ItemTable", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(conn);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_current_antigravity_sqlite_conversation_format() {
        let dir = test_dir("antigravity-db");
        let db = dir.join("11111111-1111-4111-8111-111111111111.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "create table trajectory_metadata_blob (id text primary key, data blob);\
             create table steps (idx integer primary key, step_payload blob);",
        )
        .unwrap();
        conn.execute(
            "insert into trajectory_metadata_blob (id, data) values ('main', ?1)",
            [b"meta file:///c:/Users/Test/Project".as_slice()],
        )
        .unwrap();
        conn.execute("insert into steps values (1, x'01')", [])
            .unwrap();
        conn.execute("insert into steps values (2, x'02')", [])
            .unwrap();
        drop(conn);

        let (cwd, count) = antigravity_ide_db_stats(&db);
        assert_eq!(cwd, "C:\\Users\\Test\\Project");
        assert_eq!(count, 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_orphan_antigravity_ide_artifacts_in_default_workspace() {
        let root = test_dir("antigravity-orphans");
        let brain_id = "11111111-1111-4111-8111-111111111111";
        let sidecar_id = "22222222-2222-4222-8222-222222222222";
        let db_id = "33333333-3333-4333-8333-333333333333";
        fs::create_dir_all(root.join("brain").join(brain_id)).unwrap();
        fs::create_dir_all(root.join("brain").join(db_id)).unwrap();
        fs::create_dir_all(root.join("conversations")).unwrap();
        fs::write(
            root.join("conversations")
                .join(format!("{sidecar_id}.db-wal")),
            [],
        )
        .unwrap();
        let db = root.join("conversations").join(format!("{db_id}.db"));
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("create table trajectory_metadata_blob (id text, data blob);")
            .unwrap();
        conn.execute(
            "insert into trajectory_metadata_blob values ('main', ?1)",
            [b"file:///C:/Workspace/Target".as_slice()],
        )
        .unwrap();
        drop(conn);

        let ids = antigravity_ids_for_workspace_with_index(
            &test_source(Layout::AntigravityIde),
            &root,
            "Antigravity IDE",
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(
            ids,
            HashSet::from([brain_id.to_string(), sidecar_id.to_string()])
        );
        let target_ids = antigravity_ids_for_workspace_with_index(
            &test_source(Layout::AntigravityIde),
            &root,
            "C:\\Workspace\\Target",
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(target_ids, HashSet::from([db_id.to_string()]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deletes_legacy_workspace_files_and_orphan_summary_entries() {
        let root = test_dir("antigravity-workspace");
        let conversations = root.join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let removed_id = "11111111-1111-4111-8111-111111111111";
        let orphan_id = "22222222-2222-4222-8222-222222222222";
        let keep_id = "33333333-3333-4333-8333-333333333333";
        fs::write(conversations.join(format!("{removed_id}.pb")), b"remove").unwrap();
        fs::write(conversations.join(format!("{keep_id}.pb")), b"keep").unwrap();
        let mut index = map_entry(removed_id, "file:///C:/Workspace/Target");
        index.extend(map_entry(orphan_id, "file:///C:/Workspace/Target"));
        index.extend(map_entry(keep_id, "file:///C:/Workspace/Other"));
        fs::write(root.join("agyhub_summaries_proto.pb"), index).unwrap();

        delete_antigravity_workspace(
            &test_source(Layout::Antigravity),
            &root,
            "C:\\Workspace\\Target",
        )
        .unwrap();

        assert!(!conversations.join(format!("{removed_id}.pb")).exists());
        assert!(conversations.join(format!("{keep_id}.pb")).exists());
        let remaining = fs::read(root.join("agyhub_summaries_proto.pb")).unwrap();
        let remaining_ids = protobuf_map_entries(&remaining)
            .into_iter()
            .map(|(_, id)| id)
            .collect::<HashSet<_>>();
        assert_eq!(remaining_ids, HashSet::from([keep_id.to_string()]));
        fs::remove_dir_all(root).unwrap();
    }
}

use super::*;

pub(super) fn antigravity_ide_summary(
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

pub(super) fn first_markdown_heading(path: &Path) -> Option<String> {
    BufReader::new(fs::File::open(path).ok()?)
        .lines()
        .map_while(Result::ok)
        .find_map(|line| {
            let text = line.trim().trim_start_matches('#').trim();
            (!text.is_empty()).then(|| truncate(text, MAX_TITLE_CHARS))
        })
}

pub(super) fn antigravity_ide_workspace_from_dir(dir: &Path) -> Option<String> {
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

pub(super) fn antigravity_ide_message_count(dir: &Path, conv: &Path) -> usize {
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

pub(super) fn collect_antigravity_ide_source(d: &SourceDef, root: &Path) -> Vec<SessionSummary> {
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

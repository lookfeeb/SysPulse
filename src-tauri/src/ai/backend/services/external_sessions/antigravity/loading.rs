use super::*;

pub(super) fn load_antigravity_summary_session(
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

pub(super) fn load_antigravity_ide_index_session(
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

pub(super) fn load_antigravity_ide_session(
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

pub(super) fn append_antigravity_ide_conversation(history: &mut Vec<HistoryItem>, path: &Path) {
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

pub(super) fn load_antigravity_ide_db_session(
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

pub(super) fn push_antigravity_ide_file(
    history: &mut Vec<HistoryItem>,
    dir: &Path,
    rel: &str,
    label: &str,
) {
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

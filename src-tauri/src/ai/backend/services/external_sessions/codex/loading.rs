use super::*;

pub(super) fn load_codex_virtual_session(
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

pub(super) fn load_codex_virtual_session_page(
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

pub(super) fn load_codex_archive_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
) -> anyhow::Result<IdeSession> {
    let path = codex_archive_session_path(root, session_id)
        .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
    let mut parsed = parse_codex_file(&path)?;
    if let Some(id) = codex_id_from_path(&path)? {
        if let Some(meta) = codex_metadata_for_id(root, &id)? {
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

pub(super) fn load_codex_archive_session_page(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let path = codex_archive_session_path(root, session_id)
        .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
    let metadata = if let Some(id) = codex_id_from_path(&path)? {
        codex_metadata_for_id(root, &id)?
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

pub(super) fn load_codex_index_session(
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

pub(super) fn load_codex_index_session_page(
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

pub(super) fn codex_index_fallback_session(
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

// ===== 详情 / 删除 / 路径 =====

/// 安全定位文件：拼接后 canonicalize 并校验在 root 内
fn safe_path(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.contains("..") {
        return None;
    }
    let target = root.join(rel);
    let croot = root.canonicalize().ok()?;
    let ctarget = target.canonicalize().ok()?;
    ctarget.starts_with(&croot).then_some(ctarget)
}

fn locate(hash: &str, session_id: &str) -> Option<(&'static SourceDef, PathBuf)> {
    let d = def_for(hash)?;
    let root = (d.root)()?;
    Some((d, safe_path(&root, session_id)?))
}

pub fn load_session_page(
    hash: &str,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let d = def_for(hash).ok_or_else(|| anyhow::anyhow!("未知的会话来源"))?;
    let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
    if is_virtual_session(session_id) {
        if d.source == "codex" {
            return load_codex_virtual_session_page(
                d,
                &root,
                session_id,
                hash,
                page,
                page_size,
            );
        }
        return Ok(SessionPage::from_session(
            load_session(hash, session_id)?,
            page,
            page_size,
        ));
    }

    let (d, path) = locate(hash, session_id).ok_or_else(|| anyhow::anyhow!("非法的会话路径"))?;
    match d.layout {
        Layout::File { .. } if d.source == "codex" => {
            let metadata = if let Some(id) = codex_id_from_path(&path)? {
                codex_metadata_result(&root)?.remove(&id)
            } else {
                None
            };
            let cwd_fallback = metadata
                .as_ref()
                .and_then(|meta| (!meta.cwd.is_empty()).then(|| meta.cwd.clone()));
            let title_override = metadata.as_ref().and_then(|meta| {
                (!meta.title.trim().is_empty()).then(|| codex_meta_title(meta))
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
        Layout::File { .. } if d.source == "claude" => {
            load_claude_file_page(d, &path, session_id, page, page_size)
        }
        Layout::Gemini => load_gemini_file_page(d, &path, session_id, page, page_size),
        Layout::File { .. } | Layout::Antigravity | Layout::AntigravityIde => {
            Ok(SessionPage::from_session(
                load_session(hash, session_id)?,
                page,
                page_size,
            ))
        }
    }
}

pub fn load_session(hash: &str, session_id: &str) -> anyhow::Result<IdeSession> {
    let d = def_for(hash).ok_or_else(|| anyhow::anyhow!("未知的会话来源"))?;
    let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
    if is_virtual_session(session_id) {
        return match d.source {
            "codex" => load_codex_virtual_session(d, &root, session_id, hash),
            "claude" => load_claude_history_session(d, &root, session_id, hash),
            "antigravity" => load_antigravity_summary_session(d, &root, session_id, hash),
            "antigravity-ide" => load_antigravity_ide_index_session(d, session_id, hash),
            _ => Err(anyhow::anyhow!("不支持的索引会话来源")),
        };
    }
    let (d, path) = locate(hash, session_id).ok_or_else(|| anyhow::anyhow!("非法的会话路径"))?;
    if matches!(d.layout, Layout::AntigravityIde) {
        return load_antigravity_ide_session(d, &root, &path, session_id, hash);
    }

    if matches!(d.layout, Layout::Antigravity) {
        let bytes = read_binary_file(&path).ok_or_else(|| anyhow::anyhow!("无法读取会话文件"))?;
        let mut parsed = antigravity_strings_to_parsed(&bytes);
        let index_root = if d.source == "antigravity-backup" {
            antigravity_root()
                .filter(|path| path.is_dir())
                .unwrap_or_else(|| root.clone())
        } else {
            root.clone()
        };
        if let Some(id) = path
            .file_stem()
            .and_then(|name| name.to_str())
            .and_then(extract_uuid)
        {
            if let Some((title, cwd)) = antigravity_summary_index(&index_root).get(&id).cloned() {
                if !title.trim().is_empty() {
                    parsed.title = title;
                }
                if parsed.cwd.is_empty() && !cwd.trim().is_empty() {
                    parsed.cwd = cwd;
                }
            }
        }
        if parsed.cwd.is_empty() {
            parsed.cwd = hash.strip_prefix(d.prefix).unwrap_or_default().to_string();
        }
        let mut history = Vec::new();
        if !parsed.cwd.is_empty() {
            history.push(history_item(
                "system",
                format!("工作目录：{}", parsed.cwd),
                0,
            ));
        }
        for (i, (role, text)) in parsed.blocks.iter().enumerate() {
            history.push(history_item(role, text.clone(), i + 1));
        }
        return Ok(IdeSession {
            session_id: session_id.to_string(),
            title: parsed.title,
            session_type: d.source.to_string(),
            workspace_directory: parsed.cwd,
            history,
            conversation_summary: None,
        });
    }

    let parsed = match d.layout {
        Layout::File { .. } if d.source == "codex" => {
            let mut parsed = parse_codex_file(&path)?;
            if let Some(id) = codex_id_from_path(&path)? {
                if let Some(meta) = codex_metadata_result(&root)?.remove(&id) {
                    let meta_title = codex_meta_title(&meta);
                    if parsed.cwd.is_empty() {
                        parsed.cwd = meta.cwd;
                    }
                    if !meta.title.trim().is_empty() {
                        parsed.title = meta_title;
                    }
                }
            }
            parsed
        }
        Layout::File { .. } if d.source == "claude" => parse_claude_file(&path)?,
        Layout::File { .. } => {
            let content =
                read_text_file(&path).ok_or_else(|| anyhow::anyhow!("无法读取会话文件"))?;
            (d.parse)(&content)
        }
        Layout::Gemini => {
            let mut parsed = parse_gemini_file(&path)?;
            if parsed.cwd.is_empty() {
                parsed.cwd = hash.strip_prefix(d.prefix).unwrap_or_default().to_string();
            }
            parsed
        }
        Layout::Antigravity | Layout::AntigravityIde => {
            return Err(anyhow::anyhow!("不支持的会话布局: {:?}", d.layout));
        }
    };
    Ok(parsed_to_session(d, session_id, parsed))
}

fn parsed_to_session(d: &SourceDef, session_id: &str, parsed: Parsed) -> IdeSession {
    let mut history = Vec::new();
    if !parsed.cwd.is_empty() {
        history.push(history_item(
            "system",
            format!("工作目录：{}", parsed.cwd),
            0,
        ));
    }
    for (i, (role, text)) in parsed.blocks.iter().enumerate() {
        history.push(history_item(role, text.clone(), i + 1));
    }
    IdeSession {
        session_id: session_id.to_string(),
        title: parsed.title,
        session_type: d.source.to_string(),
        workspace_directory: parsed.cwd,
        history,
        conversation_summary: None,
    }
}

fn history_item(role: &str, text: String, idx: usize) -> HistoryItem {
    HistoryItem {
        message: Message {
            role: role.to_string(),
            content: vec![ContentItem {
                content_type: "text".to_string(),
                text,
            }],
            is_hidden: false,
            id: format!("ext-{idx}"),
        },
        context_items: Vec::new(),
        editor_state: serde_json::Value::Null,
        prompt_logs: Vec::new(),
    }
}

pub fn file_path(hash: &str, session_id: &str) -> anyhow::Result<String> {
    if is_virtual_session(session_id) {
        let d = def_for(hash).ok_or_else(|| anyhow::anyhow!("未知的会话来源"))?;
        if d.source == "codex" {
            let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
            if let Some(path) = codex_archive_session_path(&root, session_id) {
                let value = path.to_string_lossy().to_string();
                return Ok(value
                    .strip_prefix(r"\\?\")
                    .map(str::to_string)
                    .unwrap_or(value));
            }
        }
        return Err(anyhow::anyhow!(
            "该会话仅来自历史索引，没有可打开的正文文件"
        ));
    }
    let (d, path) = locate(hash, session_id).ok_or_else(|| anyhow::anyhow!("非法的会话路径"))?;
    let p = match d.layout {
        Layout::File { .. } => path,
        Layout::Antigravity => path,
        Layout::AntigravityIde => path,
        Layout::Gemini => path,
    };
    // 去掉 Windows canonicalize 产生的 \\?\ 扩展长度前缀
    let s = p.to_string_lossy().to_string();
    Ok(s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s))
}

pub fn delete_session(hash: &str, session_id: &str) -> anyhow::Result<()> {
    let _guard = lock_session_mutations()?;
    delete_session_unlocked(hash, session_id)
}

fn delete_session_unlocked(hash: &str, session_id: &str) -> anyhow::Result<()> {
    let d = def_for(hash).ok_or_else(|| anyhow::anyhow!("未知的会话来源"))?;
    if is_virtual_session(session_id) {
        let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
        let result = match d.source {
            "codex" => delete_codex_virtual_session(&root, session_id),
            "claude" => delete_claude_session(&root, None, session_id),
            _ => Err(anyhow::anyhow!(
                "该来源仅提供只读索引，未执行删除"
            )),
        };
        invalidate_cache();
        return result;
    }
    if d.source == "antigravity-backup" {
        return Err(anyhow::anyhow!(
            "Antigravity 备份会话为只读数据，未执行删除"
        ));
    }
    let (d, path) = locate(hash, session_id).ok_or_else(|| anyhow::anyhow!("非法的会话路径"))?;
    let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
    if d.source == "claude" {
        let result = delete_claude_session(&root, Some(&path), session_id);
        invalidate_cache();
        return result;
    }
    if d.source == "codex" {
        let result = delete_codex_session(&root, &path);
        invalidate_cache();
        return result;
    }
    if matches!(d.layout, Layout::Antigravity | Layout::AntigravityIde) {
        let result = delete_antigravity_session(d, &root, session_id);
        invalidate_cache();
        return result;
    }
    match d.layout {
        Layout::File { .. } | Layout::Gemini => fs::remove_file(&path)?,
        Layout::Antigravity | Layout::AntigravityIde => unreachable!(),
    }
    invalidate_cache();
    Ok(())
}

/// 删除整个 workspace（该 cwd 下全部会话）
pub fn delete_workspace(hash: &str) -> anyhow::Result<()> {
    let _guard = lock_session_mutations()?;
    delete_workspace_unlocked(hash)
}

fn delete_workspace_unlocked(hash: &str) -> anyhow::Result<()> {
    let d = def_for(hash).ok_or_else(|| anyhow::anyhow!("未知的会话来源"))?;
    if d.source == "antigravity-backup" {
        return Err(anyhow::anyhow!(
            "Antigravity 备份工作区为只读数据，未执行删除"
        ));
    }
    let root = (d.root)().ok_or_else(|| anyhow::anyhow!("无法定位会话目录"))?;
    let workspace = hash.strip_prefix(d.prefix).unwrap_or_default();
    if d.source == "codex" {
        let result = delete_codex_workspace(&root, workspace);
        invalidate_cache();
        return result;
    }
    if list_sessions(hash)
        .iter()
        .any(|session| is_virtual_session(&session.session_id) && d.source != "claude")
    {
        return Err(anyhow::anyhow!(
            "该工作区包含仅索引恢复的只读会话，未执行可能不完整的批量删除"
        ));
    }
    if matches!(d.layout, Layout::Antigravity | Layout::AntigravityIde) {
        let result = delete_antigravity_workspace(d, &root, workspace);
        invalidate_cache();
        return result;
    }

    invalidate_cache();
    let keys: Vec<String> = list_sessions(hash)
        .into_iter()
        .map(|s| s.session_id)
        .collect();
    let mut errors = Vec::new();
    for k in keys {
        if let Err(error) = delete_session_unlocked(hash, &k) {
            errors.push(format!("{k}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(errors.join("；")))
    }
}

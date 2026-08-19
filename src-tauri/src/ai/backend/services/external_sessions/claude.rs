// ===== Claude =====

fn parse_claude(content: &str) -> Parsed {
    parse_claude_reader(BufReader::new(content.as_bytes()))
}

fn parse_claude_file(path: &Path) -> anyhow::Result<Parsed> {
    Ok(parse_claude_reader(BufReader::new(fs::File::open(path)?)))
}

fn load_claude_file_page(
    d: &SourceDef,
    path: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let reader = BufReader::new(fs::File::open(path)?);
    let mut blocks = PagedBlocks::new(page, page_size);
    let mut cwd = String::new();
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if cwd.is_empty() {
            if let Some(value) = value.get("cwd").and_then(|value| value.as_str()) {
                cwd = value.to_string();
            }
        }
        let message = value.get("message");
        let content = message.and_then(|message| message.get("content"));
        match value.get("type").and_then(|value| value.as_str()).unwrap_or("") {
            "user" => match content {
                Some(serde_json::Value::String(text)) => blocks.push("user", text.clone()),
                Some(serde_json::Value::Array(items)) => {
                    for item in items {
                        match item.get("type").and_then(|value| value.as_str()) {
                            Some("text") => blocks.push(
                                "user",
                                item.get("text")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("tool_result") => {
                                let role = if item
                                    .get("is_error")
                                    .and_then(|value| value.as_bool())
                                    .unwrap_or(false)
                                {
                                    "error"
                                } else {
                                    "tool_result"
                                };
                                blocks.push(role, claude_tool_result_text(item.get("content")));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            "assistant" => {
                if let Some(items) = content.and_then(|content| content.as_array()) {
                    for item in items {
                        match item.get("type").and_then(|value| value.as_str()) {
                            Some("text") => blocks.push(
                                "assistant",
                                item.get("text")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("thinking") => blocks.push(
                                "thinking",
                                item.get("thinking")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("tool_use") => {
                                let name = item
                                    .get("name")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("?");
                                let input = item
                                    .get("input")
                                    .map(|input| {
                                        serde_json::to_string_pretty(input).unwrap_or_else(|error| {
                                            format!("<工具参数序列化失败: {error}>")
                                        })
                                    })
                                    .unwrap_or_default();
                                blocks.push("tool_use", format!("工具调用：{name}\n{input}"));
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(blocks.finish(session_id, d.source, cwd, None))
}

fn parse_claude_reader(reader: impl BufRead) -> Parsed {
    let mut p = Parsed {
        cwd: String::new(),
        title: String::new(),
        created: None,
        updated: None,
        blocks: Vec::new(),
    };
    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(ts) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(iso_secs)
        {
            p.updated = Some(ts);
            if p.created.is_none() {
                p.created = Some(ts);
            }
        }
        if p.cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                p.cwd = c.to_string();
            }
        }
        let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        let msg = v.get("message");
        let content_val = msg.and_then(|m| m.get("content"));
        match ty {
            "user" => match content_val {
                Some(serde_json::Value::String(s)) => push_block(&mut p.blocks, "user", s.clone()),
                Some(serde_json::Value::Array(arr)) => {
                    for it in arr {
                        match it.get("type").and_then(|x| x.as_str()) {
                            Some("text") => push_block(
                                &mut p.blocks,
                                "user",
                                it.get("text")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("tool_result") => {
                                let txt = claude_tool_result_text(it.get("content"));
                                let role = if it
                                    .get("is_error")
                                    .and_then(|x| x.as_bool())
                                    .unwrap_or(false)
                                {
                                    "error"
                                } else {
                                    "tool_result"
                                };
                                push_block(&mut p.blocks, role, txt);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            "assistant" => {
                if let Some(arr) = content_val.and_then(|c| c.as_array()) {
                    for it in arr {
                        match it.get("type").and_then(|x| x.as_str()) {
                            Some("text") => push_block(
                                &mut p.blocks,
                                "assistant",
                                it.get("text")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("thinking") => push_block(
                                &mut p.blocks,
                                "thinking",
                                it.get("thinking")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ),
                            Some("tool_use") => {
                                let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("?");
                                let input = it
                                    .get("input")
                                    .map(|i| {
                                        serde_json::to_string_pretty(i).unwrap_or_else(|error| {
                                            format!("<工具参数序列化失败: {error}>")
                                        })
                                    })
                                    .unwrap_or_default();
                                push_block(
                                    &mut p.blocks,
                                    "tool_use",
                                    format!("工具调用：{name}\n{input}"),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    p.title = title_from(&p.blocks);
    p
}

fn claude_tool_result_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter(|it| it.get("type").and_then(|x| x.as_str()) == Some("text"))
            .filter_map(|it| it.get("text").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn scan_claude_summary(path: &Path) -> Option<ParsedSummary> {
    let size = fs::metadata(path).map(|m| m.len()).ok()?;
    if size == 0 {
        return None;
    }

    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut cwd = String::new();
    let mut first_user = String::new();
    let mut created = None;
    let mut updated = None;
    let mut message_count = 0usize;
    let mut stable_id = None;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(ts) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(iso_secs)
        {
            updated = Some(ts);
            if created.is_none() {
                created = Some(ts);
            }
        }
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                cwd = c.to_string();
            }
        }
        if stable_id.is_none() {
            stable_id = v
                .get("sessionId")
                .or_else(|| v.get("session_id"))
                .and_then(|x| x.as_str())
                .and_then(extract_uuid);
        }

        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "user" => {
                message_count += 1;
                if first_user.is_empty() {
                    first_user = claude_user_text(v.get("message").and_then(|m| m.get("content")));
                }
            }
            "assistant" => {
                message_count += 1;
            }
            _ => {}
        }
    }

    if cwd.is_empty() && message_count == 0 {
        return None;
    }

    Some(ParsedSummary {
        stable_id: stable_id.or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .and_then(extract_uuid)
        }),
        cwd,
        title: title_from_user_text(&first_user),
        created,
        updated,
        message_count,
    })
}

#[derive(Clone, Default)]
struct ClaudeSessionMeta {
    id: String,
    cwd: String,
    title: String,
    created: Option<i64>,
    updated: Option<i64>,
    user_messages: Vec<(i64, String)>,
}

fn claude_history_text(value: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(display) = value.get("display").and_then(|value| value.as_str()) {
        if !display.trim().is_empty() {
            parts.push(display.trim().to_string());
        }
    }
    if let Some(pasted) = value
        .get("pastedContents")
        .and_then(|value| value.as_object())
    {
        for item in pasted.values() {
            let Some(content) = item.get("content").and_then(|value| value.as_str()) else {
                continue;
            };
            if !content.trim().is_empty() {
                parts.push(format!("[粘贴内容]\n{}", content.trim()));
            }
        }
    }
    parts.join("\n\n")
}

fn claude_config_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config_dir) = root.parent() {
        paths.push(config_dir.join(".claude.json"));
        if config_dir.file_name().and_then(|name| name.to_str()) == Some(".claude") {
            if let Some(parent) = config_dir.parent() {
                paths.push(parent.join(".claude.json"));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn claude_scan_config_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = claude_config_paths(root);
    let Some(config_dir) = root.parent() else {
        return paths;
    };
    let backup_dir = config_dir.join("backups");
    if let Ok(entries) = fs::read_dir(backup_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".claude.json.backup."))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn claude_metadata_result(root: &Path) -> anyhow::Result<HashMap<String, ClaudeSessionMeta>> {
    let claude_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Claude 数据目录"))?;
    let mut out: HashMap<String, ClaudeSessionMeta> = HashMap::new();
    let history_path = claude_home.join("history.jsonl");
    if let Ok(file) = fs::File::open(history_path) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(id) = value
                .get("sessionId")
                .or_else(|| value.get("session_id"))
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
            else {
                continue;
            };
            let timestamp = value
                .get("timestamp")
                .and_then(|value| value.as_i64())
                .map(epoch_secs)
                .unwrap_or_default();
            let text = claude_history_text(&value);
            let entry = out.entry(id.clone()).or_insert_with(|| ClaudeSessionMeta {
                id,
                ..Default::default()
            });
            if let Some(project) = value.get("project").and_then(|value| value.as_str()) {
                if !project.trim().is_empty() {
                    entry.cwd = project.to_string();
                }
            }
            if !text.trim().is_empty() {
                if entry.title.is_empty() {
                    entry.title = value
                        .get("display")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or(&text)
                        .to_string();
                }
                entry.user_messages.push((timestamp, text));
            }
            if timestamp > 0 {
                entry.created = [entry.created, Some(timestamp)].into_iter().flatten().min();
                entry.updated = [entry.updated, Some(timestamp)].into_iter().flatten().max();
            }
        }
    }

    for config_path in claude_scan_config_paths(root) {
        let Ok(content) = fs::read_to_string(config_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(projects) = value.get("projects").and_then(|value| value.as_object()) else {
            continue;
        };
        for (project, project_meta) in projects {
            let Some(id) = project_meta
                .get("lastSessionId")
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
            else {
                continue;
            };
            let entry = out.entry(id.clone()).or_insert_with(|| ClaudeSessionMeta {
                id,
                ..Default::default()
            });
            if entry.cwd.is_empty() {
                entry.cwd = project.to_string();
            }
            if let Some(title) = project_meta
                .get("lastSessionFirstPrompt")
                .and_then(|value| value.as_str())
            {
                if entry.title.is_empty() && !title.trim().is_empty() {
                    entry.title = title.to_string();
                }
            }
            if let Some(updated) = project_meta
                .get("lastSessionModified")
                .and_then(|value| value.as_i64())
                .map(epoch_secs)
            {
                entry.updated = [entry.updated, Some(updated)].into_iter().flatten().max();
            }
        }
    }

    for entry in out.values_mut() {
        entry.user_messages.sort_by_key(|(timestamp, _)| *timestamp);
        entry.user_messages.dedup();
    }
    Ok(out)
}

fn claude_meta_title(meta: &ClaudeSessionMeta) -> String {
    let text = if !meta.title.trim().is_empty() {
        meta.title.as_str()
    } else {
        meta.user_messages
            .first()
            .map(|(_, text)| text.as_str())
            .unwrap_or_default()
    };
    title_from_user_text(text)
}

fn collect_claude_source(d: &SourceDef, root: &Path, depth: usize) -> Vec<SessionSummary> {
    let mut files = Vec::new();
    collect_files(root, "jsonl", depth, &mut files);
    files.sort();
    let mut metadata = claude_metadata_result(root).unwrap_or_default();
    let mut seen_ids = HashSet::new();
    let mut out = Vec::new();

    for file in files {
        let Some(parsed) = scan_claude_summary(&file) else {
            continue;
        };
        let stable_id = parsed.stable_id.clone();
        if stable_id
            .as_ref()
            .is_some_and(|id| !seen_ids.insert(id.clone()))
        {
            continue;
        }
        let Some(key) = rel_key(root, &file) else {
            continue;
        };
        let mut summary = summary_from_scan(d, parsed, key, &file);
        if let Some(meta) = stable_id.as_ref().and_then(|id| metadata.remove(id)) {
            if !meta.title.trim().is_empty() {
                summary.title = claude_meta_title(&meta);
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
            summary.message_count = summary.message_count.max(meta.user_messages.len());
        }
        out.push(summary);
    }

    for meta in metadata.into_values() {
        let cwd = if meta.cwd.trim().is_empty() {
            "Claude 历史".to_string()
        } else {
            meta.cwd.clone()
        };
        out.push(SessionSummary {
            session_id: virtual_session_id("history", &meta.id),
            title: claude_meta_title(&meta),
            session_type: d.source.to_string(),
            workspace_directory: cwd.clone(),
            workspace_hash: format!("{}{cwd}", d.prefix),
            message_count: meta.user_messages.len(),
            file_size: 0,
            created_at: meta.created,
            modified_at: meta.updated,
            source: d.source.to_string(),
        });
    }
    out
}

fn load_claude_history_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
    hash: &str,
) -> anyhow::Result<IdeSession> {
    let raw_id = virtual_session_key(session_id, "history")
        .ok_or_else(|| anyhow::anyhow!("无法识别 Claude 历史会话"))?;
    let id = extract_uuid(raw_id).ok_or_else(|| anyhow::anyhow!("Claude 会话 ID 无效"))?;
    let meta = claude_metadata_result(root)?
        .remove(&id)
        .ok_or_else(|| anyhow::anyhow!("Claude 历史记录已不存在，请刷新列表"))?;
    let cwd = if meta.cwd.trim().is_empty() {
        hash.strip_prefix(d.prefix)
            .unwrap_or("Claude 历史")
            .to_string()
    } else {
        meta.cwd.clone()
    };
    let mut history = vec![history_item(
        "system",
        "Claude 会话正文已不在 projects 目录；以下内容由 history.jsonl 和 ~/.claude.json 恢复，只包含用户输入与最近会话元数据。".to_string(),
        0,
    )];
    for (_, text) in &meta.user_messages {
        history.push(history_item("user", text.clone(), history.len()));
    }
    Ok(IdeSession {
        session_id: session_id.to_string(),
        title: claude_meta_title(&meta),
        session_type: d.source.to_string(),
        workspace_directory: cwd,
        history,
        conversation_summary: None,
    })
}

fn claude_session_id_from_file(path: &Path) -> anyhow::Result<Option<String>> {
    let file = fs::File::open(path)?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = value
            .get("sessionId")
            .or_else(|| value.get("session_id"))
            .and_then(|value| value.as_str())
            .and_then(extract_uuid)
        {
            return Ok(Some(id));
        }
    }
    Ok(path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(extract_uuid))
}

fn rewrite_claude_history_without_id(path: &Path, id: &str) -> anyhow::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Claude history 路径不是文件: {}",
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
                    .get("sessionId")
                    .or_else(|| value.get("session_id"))
                    .and_then(|value| value.as_str())
                    .and_then(extract_uuid)
            })
            .is_some_and(|value| value == id);
        if remove {
            changed = true;
        } else {
            filtered.push_str(line);
        }
    }
    if changed {
        crate::ai::backend::utils::fs::atomic_write(path, &filtered, "Claude 会话索引")
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

fn rewrite_claude_config_without_id(path: &Path, id: &str) -> anyhow::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Claude 配置路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let content = fs::read_to_string(path)?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| anyhow::anyhow!("解析 Claude 配置失败 {}: {error}", path.display()))?;
    let Some(projects) = value
        .get_mut("projects")
        .and_then(|value| value.as_object_mut())
    else {
        return Ok(());
    };
    let mut changed = false;
    for project in projects.values_mut() {
        let Some(project) = project.as_object_mut() else {
            continue;
        };
        let matches_id = ["lastSessionId", "lastHintSessionId"].iter().any(|key| {
            project
                .get(*key)
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
                .is_some_and(|value| value == id)
        });
        if !matches_id {
            continue;
        }
        for key in [
            "lastSessionId",
            "lastSessionFirstPrompt",
            "lastSessionModified",
            "lastHintSessionId",
        ] {
            changed |= project.remove(key).is_some();
        }
    }
    if !changed {
        return Ok(());
    }
    let mut output = serde_json::to_vec_pretty(&value)?;
    output.push(b'\n');
    crate::ai::backend::utils::fs::atomic_write_bytes(path, &output, "Claude 配置")
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

fn delete_claude_session(root: &Path, path: Option<&Path>, session_id: &str) -> anyhow::Result<()> {
    let id = virtual_session_key(session_id, "history")
        .and_then(extract_uuid)
        .or_else(|| path.and_then(|path| claude_session_id_from_file(path).ok().flatten()))
        .ok_or_else(|| anyhow::anyhow!("无法识别 Claude 会话 ID，未执行不完整删除"))?;
    let claude_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Claude 数据目录"))?;
    rewrite_claude_history_without_id(&claude_home.join("history.jsonl"), &id)?;
    for config in claude_config_paths(root) {
        rewrite_claude_config_without_id(&config, &id)?;
    }
    if let Some(path) = path {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => fs::remove_file(path)?,
            Ok(_) => {
                return Err(anyhow::anyhow!(
                    "Claude 会话路径不是文件: {}",
                    path.display()
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if claude_metadata_result(root)?.contains_key(&id) {
        return Err(anyhow::anyhow!(
            "Claude 会话索引仍有残留，已保留可重试状态: {id}"
        ));
    }
    Ok(())
}

fn claude_user_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter(|it| it.get("type").and_then(|x| x.as_str()) == Some("text"))
            .filter_map(|it| it.get("text").and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod claude_tests {
    use super::*;

    fn no_test_root() -> Option<PathBuf> {
        None
    }

    fn test_source() -> SourceDef {
        SourceDef {
            prefix: "claude-test:",
            source: "claude",
            root: no_test_root,
            layout: Layout::File { depth: 4 },
            parse: parse_claude,
            scan: scan_claude_summary,
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn streams_claude_page_and_counts_expanded_content_blocks() {
        let dir = test_dir("claude-page");
        let path = dir.join("session.jsonl");
        let lines = [
            serde_json::json!({
                "type": "user",
                "cwd": "C:\\Workspace\\Claude",
                "message": { "content": "第一条" }
            }),
            serde_json::json!({
                "type": "assistant",
                "message": { "content": [
                    { "type": "thinking", "thinking": "第二条" },
                    { "type": "text", "text": "第三条" }
                ] }
            }),
            serde_json::json!({
                "type": "user",
                "message": { "content": "第四条" }
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

        let page = load_claude_file_page(&test_source(), &path, "session", 2, 2).unwrap();
        assert_eq!(page.total_messages, 5);
        assert_eq!(page.title, "第一条");
        assert_eq!(page.history.len(), 2);
        assert_eq!(page.history[0].message.content[0].text, "第二条");
        assert_eq!(page.history[1].message.content[0].text, "第三条");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restores_claude_history_without_project_jsonl() {
        let home = test_dir("claude-restore");
        let claude_home = home.join(".claude");
        let root = claude_home.join("projects");
        fs::create_dir_all(&root).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        fs::write(
            claude_home.join("history.jsonl"),
            format!(
                "{{\"display\":\"恢复 Claude 输入\",\"pastedContents\":{{\"1\":{{\"content\":\"粘贴正文\"}}}},\"project\":\"C:\\\\Workspace\\\\Claude\",\"sessionId\":\"{id}\",\"timestamp\":1700000000000}}\n"
            ),
        )
        .unwrap();
        fs::write(
            home.join(".claude.json"),
            format!(
                "{{\"projects\":{{\"C:\\\\Workspace\\\\Claude\":{{\"lastSessionId\":\"{id}\",\"lastSessionFirstPrompt\":\"最近输入\",\"lastSessionModified\":1700000100000}}}}}}"
            ),
        )
        .unwrap();

        let summaries = collect_claude_source(&test_source(), &root, 4);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].session_id, virtual_session_id("history", id));
        assert_eq!(summaries[0].title, "恢复 Claude 输入");
        assert_eq!(summaries[0].message_count, 1);

        let loaded = load_claude_history_session(
            &test_source(),
            &root,
            &summaries[0].session_id,
            &summaries[0].workspace_hash,
        )
        .unwrap();
        assert_eq!(loaded.history.len(), 2);
        let text = &loaded.history[1].message.content[0].text;
        assert!(text.contains("恢复 Claude 输入"));
        assert!(text.contains("粘贴正文"));
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn deletes_claude_body_and_all_known_indexes_together() {
        let home = test_dir("claude-delete");
        let claude_home = home.join(".claude");
        let root = claude_home.join("projects");
        fs::create_dir_all(&root).unwrap();
        let id = "22222222-2222-4222-8222-222222222222";
        let body = root.join(format!("{id}.jsonl"));
        fs::write(
            &body,
            format!(
                "{{\"sessionId\":\"{id}\",\"type\":\"user\",\"cwd\":\"C:\\\\Workspace\",\"message\":{{\"content\":\"body\"}}}}\n"
            ),
        )
        .unwrap();
        fs::write(
            claude_home.join("history.jsonl"),
            format!("{{\"sessionId\":\"{id}\",\"display\":\"history\",\"project\":\"C:\\\\Workspace\",\"timestamp\":1700000000000}}\n"),
        )
        .unwrap();
        fs::write(
            home.join(".claude.json"),
            format!(
                "{{\"projects\":{{\"C:\\\\Workspace\":{{\"lastSessionId\":\"{id}\",\"lastSessionFirstPrompt\":\"prompt\",\"lastSessionModified\":1700000100000}}}}}}"
            ),
        )
        .unwrap();

        delete_claude_session(&root, Some(&body), &format!("{id}.jsonl")).unwrap();
        assert!(!body.exists());
        assert!(!fs::read_to_string(claude_home.join("history.jsonl"))
            .unwrap()
            .contains(id));
        assert!(!fs::read_to_string(home.join(".claude.json"))
            .unwrap()
            .contains(id));
        assert!(claude_metadata_result(&root).unwrap().is_empty());
        fs::remove_dir_all(home).unwrap();
    }
}

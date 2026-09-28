use super::*;

pub(super) fn load_claude_file_page(
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
        match value
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or("")
        {
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
                                        serde_json::to_string_pretty(input).unwrap_or_else(
                                            |error| format!("<工具参数序列化失败: {error}>"),
                                        )
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

pub(super) fn load_claude_history_session(
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

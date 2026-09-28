use super::*;

// ===== Claude =====

pub(super) fn parse_claude(content: &str) -> Parsed {
    parse_claude_reader(BufReader::new(content.as_bytes()))
}

pub(super) fn parse_claude_file(path: &Path) -> anyhow::Result<Parsed> {
    Ok(parse_claude_reader(BufReader::new(fs::File::open(path)?)))
}

pub(super) fn parse_claude_reader(reader: impl BufRead) -> Parsed {
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

pub(super) fn claude_tool_result_text(content: Option<&serde_json::Value>) -> String {
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

pub(super) fn claude_user_text(content: Option<&serde_json::Value>) -> String {
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

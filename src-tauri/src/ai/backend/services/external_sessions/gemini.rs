// ===== Gemini CLI（归入 Antigravity 平台） =====

fn gemini_content_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                serde_json::Value::String(text) => Some(text.as_str()),
                serde_json::Value::Object(value) => value
                    .get("text")
                    .or_else(|| value.get("content"))
                    .and_then(|value| value.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn apply_gemini_value(parsed: &mut Parsed, value: &serde_json::Value) {
    for key in ["startTime", "timestamp", "lastUpdated"] {
        if let Some(timestamp) = value
            .get(key)
            .and_then(|value| value.as_str())
            .and_then(iso_secs)
        {
            parsed.created = [parsed.created, Some(timestamp)]
                .into_iter()
                .flatten()
                .min();
            parsed.updated = [parsed.updated, Some(timestamp)]
                .into_iter()
                .flatten()
                .max();
        }
    }
    let role = match value.get("type").and_then(|value| value.as_str()) {
        Some("user") => "user",
        Some("gemini") => "assistant",
        Some("error") => "error",
        Some("info") => "system",
        _ => return,
    };
    push_block(
        &mut parsed.blocks,
        role,
        gemini_content_text(value.get("content")),
    );
}

fn parse_gemini(content: &str) -> Parsed {
    parse_gemini_reader(BufReader::new(content.as_bytes()))
}

fn parse_gemini_file(path: &Path) -> anyhow::Result<Parsed> {
    let mut parsed = parse_gemini_reader(BufReader::new(fs::File::open(path)?));
    if parsed.cwd.is_empty() {
        parsed.cwd = gemini_workspace_from_path(path);
    }
    Ok(parsed)
}

fn load_gemini_file_page(
    d: &SourceDef,
    path: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
) -> anyhow::Result<SessionPage> {
    let reader = BufReader::new(fs::File::open(path)?);
    let mut blocks = PagedBlocks::new(page, page_size);
    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let role = match value.get("type").and_then(|value| value.as_str()) {
            Some("user") => "user",
            Some("gemini") => "assistant",
            Some("error") => "error",
            Some("info") => "system",
            _ => continue,
        };
        blocks.push(role, gemini_content_text(value.get("content")));
    }
    Ok(blocks.finish(
        session_id,
        d.source,
        gemini_workspace_from_path(path),
        None,
    ))
}

fn parse_gemini_reader(reader: impl BufRead) -> Parsed {
    let mut parsed = Parsed {
        cwd: String::new(),
        title: String::new(),
        created: None,
        updated: None,
        blocks: Vec::new(),
    };
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        apply_gemini_value(&mut parsed, &value);
    }
    parsed.title = title_from(&parsed.blocks);
    parsed
}

fn gemini_workspace_from_path(path: &Path) -> String {
    path.parent()
        .and_then(Path::parent)
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("Gemini CLI / {name}"))
        .unwrap_or_else(|| "Gemini CLI".to_string())
}

fn scan_gemini_summary(path: &Path) -> Option<ParsedSummary> {
    if fs::metadata(path).map(|metadata| metadata.len()).ok()? == 0 {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut stable_id = None;
    let mut first_user = String::new();
    let mut created = None;
    let mut updated = None;
    let mut message_count = 0usize;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if stable_id.is_none() {
            stable_id = value
                .get("sessionId")
                .and_then(|value| value.as_str())
                .and_then(extract_uuid);
        }
        for key in ["startTime", "timestamp", "lastUpdated"] {
            if let Some(timestamp) = value
                .get(key)
                .and_then(|value| value.as_str())
                .and_then(iso_secs)
            {
                created = [created, Some(timestamp)].into_iter().flatten().min();
                updated = [updated, Some(timestamp)].into_iter().flatten().max();
            }
        }
        match value.get("type").and_then(|value| value.as_str()) {
            Some("user") => {
                message_count += 1;
                if first_user.is_empty() {
                    first_user = gemini_content_text(value.get("content"));
                }
            }
            Some("gemini" | "error" | "info") => message_count += 1,
            _ => {}
        }
    }

    if stable_id.is_none() && message_count == 0 {
        return None;
    }
    Some(ParsedSummary {
        stable_id,
        cwd: gemini_workspace_from_path(path),
        title: title_from_user_text(&first_user),
        created,
        updated,
        message_count,
    })
}

#[cfg(test)]
mod gemini_tests {
    use super::*;

    fn no_test_root() -> Option<PathBuf> {
        None
    }

    fn test_source() -> SourceDef {
        SourceDef {
            prefix: "gemini-test:",
            source: "gemini",
            root: no_test_root,
            layout: Layout::Gemini,
            parse: parse_gemini,
            scan: scan_gemini_summary,
        }
    }

    #[test]
    fn parses_gemini_cli_header_and_messages() {
        let content = concat!(
            "{\"sessionId\":\"11111111-1111-4111-8111-111111111111\",\"startTime\":\"2026-07-01T00:00:00Z\",\"lastUpdated\":\"2026-07-01T00:01:00Z\"}\n",
            "{\"type\":\"user\",\"timestamp\":\"2026-07-01T00:00:10Z\",\"content\":[{\"text\":\"修复登录问题\"}]}\n",
            "{\"type\":\"gemini\",\"timestamp\":\"2026-07-01T00:00:20Z\",\"content\":\"已完成\"}\n",
        );
        let parsed = parse_gemini(content);
        assert_eq!(parsed.title, "修复登录问题");
        assert_eq!(parsed.blocks.len(), 2);
        assert!(parsed.created.is_some());
        assert!(parsed.updated > parsed.created);
    }

    #[test]
    fn streams_gemini_page_with_stable_total() {
        let dir = std::env::temp_dir().join(format!(
            "kirohub-gemini-page-{}",
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("project-a").join("chats").join("session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = concat!(
            "{\"type\":\"user\",\"content\":\"第一条\"}\n",
            "{\"type\":\"gemini\",\"content\":\"第二条\"}\n",
            "{\"type\":\"error\",\"content\":\"第三条\"}\n",
            "{\"type\":\"info\",\"content\":\"第四条\"}\n",
        );
        fs::write(&path, content).unwrap();

        let page = load_gemini_file_page(&test_source(), &path, "session", 3, 2).unwrap();
        assert_eq!(page.total_messages, 5);
        assert_eq!(page.history.len(), 1);
        assert_eq!(page.history[0].message.content[0].text, "第四条");
        fs::remove_dir_all(dir).unwrap();
    }
}

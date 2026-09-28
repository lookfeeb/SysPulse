use super::*;

pub(super) fn claude_summary_cache() -> &'static SummaryCache {
    static CACHE: OnceLock<SummaryCache> = OnceLock::new();
    CACHE.get_or_init(SummaryCache::default)
}

pub(super) fn scan_claude_summary(path: &Path) -> Option<ParsedSummary> {
    cached_summary(claude_summary_cache(), path, scan_claude_summary_uncached)
}

pub(super) fn scan_claude_summary_uncached(path: &Path) -> Option<ParsedSummary> {
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

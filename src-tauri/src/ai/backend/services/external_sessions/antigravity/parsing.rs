use super::*;

pub(super) fn parse_antigravity(content: &str) -> Parsed {
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

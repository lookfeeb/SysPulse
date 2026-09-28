use super::*;
use std::io::{Seek, SeekFrom};
use std::sync::Arc;

const MAX_INDEXED_MESSAGES: usize = 1_000_000;
const MAX_CACHED_SESSIONS: usize = 16;

struct PageIndex {
    stamp: FileStamp,
    offsets: Vec<u64>,
    cwd: String,
    title: Option<String>,
}

fn page_indexes() -> &'static Mutex<HashMap<PathBuf, Arc<PageIndex>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<PageIndex>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn message(event: &CodexEnvelope<'_>) -> Option<(&'static str, String)> {
    let payload = event.payload.as_ref()?;
    let result = match (event.kind.as_deref(), payload.kind.as_deref()) {
        (Some("event_msg"), Some("user_message")) => {
            ("user", payload.message.as_deref()?.to_string())
        }
        (Some("response_item"), Some("message"))
            if payload.role.as_deref() == Some("assistant") =>
        {
            let text = payload
                .content
                .iter()
                .filter(|item| matches!(item.kind.as_deref(), Some("output_text" | "input_text")))
                .filter_map(|item| item.text.as_deref())
                .collect::<String>();
            ("assistant", text)
        }
        _ => return None,
    };
    (!result.1.trim().is_empty()).then_some(result)
}

fn index_file(path: &Path) -> anyhow::Result<Option<Arc<PageIndex>>> {
    let Some(stamp) = file_stamp(path) else {
        return Ok(None);
    };
    let key = file_cache_key(path);
    if let Ok(cache) = page_indexes().lock() {
        if let Some(index) = cache.get(&key).filter(|index| index.stamp == stamp) {
            return Ok(Some(index.clone()));
        }
    }
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut index = PageIndex {
        stamp,
        offsets: Vec::new(),
        cwd: String::new(),
        title: None,
    };
    let mut offset = 0;
    let mut line = String::new();
    loop {
        line.clear();
        let size = reader.read_line(&mut line)?;
        if size == 0 {
            break;
        }
        let current = offset;
        offset += size as u64;
        let Ok(event) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        if event.kind.as_deref() == Some("session_meta") {
            if let Some(cwd) = event.payload.as_ref().and_then(|p| p.cwd.as_deref()) {
                index.cwd = cwd.to_string();
            }
        }
        if let Some((role, text)) = message(&event) {
            if index.offsets.len() == MAX_INDEXED_MESSAGES {
                return Ok(None);
            }
            index.offsets.push(current);
            if role == "user" && index.title.is_none() {
                index.title = meaningful_line(&text).map(|line| truncate(&line, MAX_TITLE_CHARS));
            }
        }
    }
    if file_stamp(path) != Some(stamp) {
        return Ok(None);
    }
    let index = Arc::new(index);
    if let Ok(mut cache) = page_indexes().lock() {
        // 全局偏移量总量上限约 8 MB；不缓存正文或完整消息。
        if cache.len() >= MAX_CACHED_SESSIONS
            || cache
                .values()
                .map(|entry| entry.offsets.len())
                .sum::<usize>()
                + index.offsets.len()
                > MAX_INDEXED_MESSAGES
        {
            cache.clear();
        }
        cache.insert(key, index.clone());
    }
    Ok(Some(index))
}

pub(super) fn load_codex_file_page(
    d: &SourceDef,
    path: &Path,
    session_id: &str,
    page: usize,
    page_size: usize,
    cwd_fallback: Option<String>,
    title_override: Option<String>,
) -> anyhow::Result<SessionPage> {
    let Some(index) = index_file(path)? else {
        return load_codex_file_page_uncached(
            d,
            path,
            session_id,
            page,
            page_size,
            cwd_fallback,
            title_override,
        );
    };
    let cwd = if index.cwd.is_empty() {
        cwd_fallback.unwrap_or_default()
    } else {
        index.cwd.clone()
    };
    let mut blocks = PagedBlocks::new(page, page_size);
    blocks.raw_count = index.offsets.len();
    blocks.title = index.title.clone();
    let prefix = usize::from(!cwd.is_empty());
    let start = blocks.start.saturating_sub(prefix).min(index.offsets.len());
    let end = blocks.end.saturating_sub(prefix).min(index.offsets.len());
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut line = String::new();
    for raw_index in start..end {
        reader.seek(SeekFrom::Start(index.offsets[raw_index]))?;
        line.clear();
        reader.read_line(&mut line)?;
        let event = serde_json::from_str::<CodexEnvelope<'_>>(&line)?;
        let (role, text) =
            message(&event).ok_or_else(|| anyhow::anyhow!("Codex 正文已变化，请重新加载"))?;
        blocks.retained.push((raw_index, role.to_string(), text));
    }
    if file_stamp(path) != Some(index.stamp) {
        return Err(anyhow::anyhow!("Codex 正文正在更新，请重新加载"));
    }
    Ok(blocks.finish(session_id, d.source, cwd, title_override))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_pages_match_streaming_and_refresh_after_append() {
        let path =
            std::env::temp_dir().join(format!("syspulse-pages-{}.jsonl", uuid::Uuid::new_v4()));
        let mut content =
            String::from("{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"C:/项目\"}}\n");
        for n in 0..70 {
            content.push_str(&format!("{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"问题 {n}\"}}}}\n"));
        }
        content.push_str("invalid json\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"  \"}}\n");
        fs::write(&path, &content).unwrap();
        let d = &SOURCES[0];
        for page in [0, 1, 2, 3, 4, 100, usize::MAX] {
            let indexed = load_codex_file_page(d, &path, "test", page, 25, None, None).unwrap();
            let streamed =
                load_codex_file_page_uncached(d, &path, "test", page, 25, None, None).unwrap();
            assert_eq!(
                serde_json::to_value(indexed).unwrap(),
                serde_json::to_value(streamed).unwrap()
            );
        }
        let first = index_file(&path).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &index_file(&path).unwrap().unwrap()));
        content.push_str("{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"回答\"}]}}\n");
        fs::write(&path, content).unwrap();
        let updated = index_file(&path).unwrap().unwrap();
        assert!(!Arc::ptr_eq(&first, &updated));
        assert_eq!(updated.offsets.len(), 71);
        fs::remove_file(path).unwrap();
    }
}

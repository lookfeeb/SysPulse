fn iso_secs(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp())
}

fn epoch_secs(mut value: i64) -> i64 {
    while value.abs() >= 100_000_000_000 {
        value /= 1_000;
    }
    value
}

fn virtual_session_id(kind: &str, id: &str) -> String {
    format!("@{kind}/{id}")
}

fn virtual_session_key<'a>(session_id: &'a str, kind: &str) -> Option<&'a str> {
    session_id.strip_prefix(&format!("@{kind}/"))
}

fn is_virtual_session(session_id: &str) -> bool {
    session_id.starts_with('@')
}

fn truncate(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() > n {
        t.chars().take(n).collect::<String>() + "…"
    } else {
        t.to_string()
    }
}

/// 从 user 文本提炼标题：跳过标题/标签/代码块/文件路径/上下文样板，
/// 优先取 "My request" 标记之后的首个有意义行。
fn title_from(blocks: &[(String, String)]) -> String {
    blocks
        .iter()
        .filter(|(r, _)| r == "user")
        .find_map(|(_, t)| meaningful_line(t))
        .map(|l| truncate(&l, MAX_TITLE_CHARS))
        .unwrap_or_else(|| "未命名会话".to_string())
}

fn title_from_any(blocks: &[(String, String)]) -> String {
    blocks
        .iter()
        .find_map(|(_, t)| meaningful_line(t))
        .map(|l| truncate(&l, MAX_TITLE_CHARS))
        .unwrap_or_else(|| "未命名会话".to_string())
}

fn title_from_user_text(text: &str) -> String {
    meaningful_line(text)
        .map(|l| truncate(&l, MAX_TITLE_CHARS))
        .unwrap_or_else(|| "未命名会话".to_string())
}

/// 一行是否“有意义”（可作标题）
fn is_meaningful(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty()
        || t.starts_with('#')
        || t.starts_with('<')
        || t.starts_with("```")
        || t.starts_with("//")
    {
        return false;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("context") || lower.contains("context from") || lower.starts_with("system")
    {
        return false;
    }
    // 纯文件路径行（盘符或以 / 开头且不含空格）
    if (t.contains(":\\") || t.starts_with('/')) && !t.contains(' ') {
        return false;
    }
    true
}

fn meaningful_line(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    // 优先：My request 标记之后的首个有意义行
    for (i, l) in lines.iter().enumerate() {
        let lt = l.trim();
        if lt.starts_with("## My request")
            || lt.starts_with("# My request")
            || lt.contains("My request for")
        {
            if let Some(n) = lines.iter().skip(i + 1).find(|n| is_meaningful(n)) {
                return Some(n.trim().to_string());
            }
        }
    }
    lines
        .iter()
        .find(|l| is_meaningful(l))
        .map(|l| l.trim().to_string())
}

fn read_text_file(path: &Path) -> Option<String> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn read_binary_file(path: &Path) -> Option<Vec<u8>> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return None;
    }
    fs::read(path).ok()
}

fn metadata_secs(path: &Path, created: bool) -> Option<i64> {
    let meta = fs::metadata(path).ok()?;
    let time = if created {
        meta.created().ok()?
    } else {
        meta.modified().ok()?
    };
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// 递归收集指定扩展名文件（限定深度）
fn collect_files(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, ext, depth - 1, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
            out.push(p);
        }
    }
}

fn push_block(blocks: &mut Vec<(String, String)>, role: &str, text: String) {
    if !text.trim().is_empty() {
        blocks.push((role.to_string(), text));
    }
}

fn printable_strings(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .split(|c: char| c.is_control())
        .map(str::trim)
        .filter(|s| s.chars().count() >= 4)
        .filter(|s| !s.contains('\u{fffd}'))
        .map(str::to_string)
        .collect()
}

fn clean_file_uri(value: &str) -> String {
    let raw = value
        .strip_prefix("file:///")
        .or_else(|| value.strip_prefix("file://"))
        .unwrap_or(value);
    let decoded = urlencoding::decode(raw)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| raw.to_string());
    let mut s = decoded.replace('/', "\\");
    if s.starts_with("c:\\") {
        s.replace_range(0..1, "C");
    }
    s
}

fn clean_file_uri_at(value: &str) -> Option<String> {
    let pos = value.find("file://")?;
    let tail = &value[pos..];
    let end = tail
        .char_indices()
        .find_map(|(i, c)| {
            matches!(
                c,
                '"' | '\'' | '<' | '>' | ')' | ']' | '}' | ' ' | '\n' | '\r' | '\t'
            )
            .then_some(i)
        })
        .unwrap_or(tail.len());
    Some(normalize_workspace_path(clean_file_uri(&tail[..end])))
}

fn normalize_workspace_path(value: String) -> String {
    let mut path = value
        .split([')', '）', '：'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\'', '`', '。', ',', '，'])
        .to_string();
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        let mut chars = path.chars();
        if let Some(first) = chars.next() {
            path.replace_range(0..first.len_utf8(), &first.to_ascii_uppercase().to_string());
        }
    }
    let leaf = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if leaf.contains('.') {
        if let Some((parent, _)) = path.rsplit_once(['\\', '/']) {
            return parent.to_string();
        }
    }
    path
}

fn extract_uuid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 36 {
        return None;
    }
    for i in 0..=bytes.len() - 36 {
        let b = &bytes[i..i + 36];
        let hyphen_ok = b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-';
        if hyphen_ok
            && b.iter()
                .enumerate()
                .all(|(idx, c)| matches!(idx, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit())
        {
            return std::str::from_utf8(b).ok().map(str::to_string);
        }
    }
    None
}

fn clean_proto_text(value: &str) -> String {
    value
        .trim_matches(|c: char| !is_proto_text_boundary_char(c))
        .trim()
        .to_string()
}

fn is_proto_text_boundary_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c.is_ascii_whitespace()
        || is_cjk_text_char(c)
        || matches!(
            c,
            '/' | '\\'
                | ':'
                | '.'
                | '-'
                | '_'
                | ' '
                | '%'
                | '#'
                | '@'
                | '&'
                | '+'
                | '='
                | '?'
                | '!'
                | ','
                | ';'
                | '\''
                | '"'
                | '`'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '*'
                | '|'
                | '~'
                | '$'
                | '，'
                | '。'
                | '、'
                | '？'
                | '！'
                | '：'
                | '；'
                | '（'
                | '）'
                | '【'
                | '】'
                | '《'
                | '》'
                | '“'
                | '”'
                | '‘'
                | '’'
        )
}

fn is_cjk_text_char(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
            | 0xFF00..=0xFFEF
    )
}

fn is_common_proto_text_char(c: char) -> bool {
    c.is_ascii_graphic() || c.is_ascii_whitespace() || is_cjk_text_char(c)
}

fn looks_binary_token(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return true;
    }
    let has_space = chars.iter().any(|c| c.is_whitespace());
    let has_cjk = chars.iter().any(|c| is_cjk_text_char(*c));
    let has_separator = chars
        .iter()
        .any(|c| matches!(c, '/' | '\\' | ':' | '.' | '-' | '_' | '#'));
    if !has_space && !has_cjk && !has_separator && chars.len() < 24 {
        return true;
    }

    let compact_base64_chars = chars
        .iter()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
        .count();
    !has_space && !has_cjk && chars.len() >= 16 && compact_base64_chars * 100 / chars.len() >= 92
}

fn is_readable_proto_text(value: &str) -> bool {
    let s = value.trim();
    let len = s.chars().count();
    if len < 12
        || extract_uuid(s).is_some()
        || s == "master"
        || s.starts_with("https://")
        || s.starts_with("http://")
    {
        return false;
    }
    if s.starts_with("file://") || looks_base64_blob(s) || looks_binary_token(s) {
        return false;
    }
    if s.chars().any(|c| !is_common_proto_text_char(c)) {
        return false;
    }

    let cjk_count = s.chars().filter(|c| is_cjk_text_char(*c)).count();
    let whitespace_count = s.chars().filter(|c| c.is_whitespace()).count();
    let has_path_or_file = s.contains(":\\")
        || s.contains(":/")
        || s.contains(".rs")
        || s.contains(".ts")
        || s.contains(".tsx")
        || s.contains(".md")
        || s.contains(".json")
        || s.contains('/')
        || s.contains('\\');
    let has_sentence_punctuation = s.chars().any(|c| {
        matches!(
            c,
            '.' | ',' | '?' | '!' | ':' | ';' | '，' | '。' | '？' | '！' | '：' | '；'
        )
    });
    let has_code_punctuation = s
        .chars()
        .any(|c| matches!(c, '{' | '}' | '(' | ')' | '[' | ']' | '<' | '>' | '=' | '#'));

    cjk_count >= 2
        || whitespace_count >= 2
        || has_path_or_file
        || has_sentence_punctuation
        || (has_code_punctuation && len >= 24)
}

#[cfg(test)]
mod external_session_size_tests {
    use super::*;

    #[test]
    fn scans_and_reads_sessions_larger_than_the_previous_limit() {
        const PREVIOUS_LIMIT_BYTES: usize = 50 * 1024 * 1024;

        let dir = std::env::temp_dir().join(format!(
            "kirohub-unlimited-session-size-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large-session.jsonl");
        let id = "11111111-1111-4111-8111-111111111111";
        let header = format!(
            "{{\"type\":\"session_meta\",\"cwd\":\"C:\\\\Workspace\\\\Unlimited\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:\\\\Workspace\\\\Unlimited\"}}}}\n\
             {{\"type\":\"user\",\"cwd\":\"C:\\\\Workspace\\\\Unlimited\",\"sessionId\":\"{id}\",\"message\":{{\"content\":\"Claude 大会话\"}}}}\n\
             {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"Codex 大会话\"}}}}\n"
        );
        let padding = format!(
            "{{\"type\":\"padding\",\"value\":\"{}\"}}\n",
            "x".repeat(64 * 1024)
        );
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(header.as_bytes()).unwrap();
        let mut written = header.len();
        while written <= PREVIOUS_LIMIT_BYTES {
            file.write_all(padding.as_bytes()).unwrap();
            written += padding.len();
        }
        drop(file);

        let file_size = fs::metadata(&path).unwrap().len() as usize;
        assert!(file_size > PREVIOUS_LIMIT_BYTES);

        let codex = scan_codex_summary(&path).expect("Codex 大会话应被扫描");
        assert_eq!(codex.cwd, r"C:\Workspace\Unlimited");
        assert_eq!(codex.title, "Codex 大会话");

        let claude = scan_claude_summary(&path).expect("Claude 大会话应被扫描");
        assert_eq!(claude.cwd, r"C:\Workspace\Unlimited");
        assert_eq!(claude.title, "Claude 大会话");

        assert_eq!(read_text_file(&path).unwrap().len(), file_size);
        assert_eq!(read_binary_file(&path).unwrap().len(), file_size);

        fs::remove_dir_all(dir).unwrap();
    }
}

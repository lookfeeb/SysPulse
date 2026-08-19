// ===== 列表 =====

/// 收集所有外部来源的 workspace 标识（codex:/claude: 按 cwd，antigravity 单组）
pub fn list_workspaces() -> Vec<String> {
    let mut out = Vec::new();
    for s in collect_all() {
        if !out.contains(&s.workspace_hash) {
            out.push(s.workspace_hash.clone());
        }
    }
    out
}

/// 列出某 workspace 下的会话
pub fn list_sessions(hash: &str) -> Vec<SessionSummary> {
    collect_all()
        .into_iter()
        .filter(|s| s.workspace_hash == hash)
        .collect()
}

pub fn list_all_sessions() -> Vec<SessionSummary> {
    collect_all()
}

/// 扫描已注册的外部来源，产出全部会话摘要
fn collect_all() -> Vec<SessionSummary> {
    if let Ok(cache) = CACHE.lock() {
        if let Some((t, v)) = cache.as_ref() {
            if t.elapsed() < CACHE_TTL {
                return v.clone();
            }
        }
    }
    let fresh = scan_all();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), fresh.clone()));
    }
    fresh
}

fn scan_all() -> Vec<SessionSummary> {
    // 每个来源各起一个线程并发扫描，随注册表自动伸缩
    let mut out: Vec<SessionSummary> = std::thread::scope(|s| {
        let handles: Vec<_> = SOURCES
            .iter()
            .map(|d| s.spawn(move || collect_source(d)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    out = dedupe_external_sessions(out);
    out.sort_by_key(|item| std::cmp::Reverse(item.modified_at.unwrap_or(0)));
    out
}

/// 按来源定义扫描其根目录，产出会话摘要
fn collect_source(d: &SourceDef) -> Vec<SessionSummary> {
    let Some(root) = (d.root)() else {
        return Vec::new();
    };
    let root_available = root.is_dir()
        || (d.source == "codex"
            && codex_archive_root(&root).is_some_and(|path| path.is_dir()));
    if !root_available {
        return Vec::new();
    }
    let mut out = Vec::new();
    match d.layout {
        Layout::File { depth } => {
            if d.source == "codex" {
                return collect_codex_source(d, &root, depth);
            }
            if d.source == "claude" {
                return collect_claude_source(d, &root, depth);
            }
            let mut files = Vec::new();
            collect_files(&root, "jsonl", depth, &mut files);
            for f in files {
                let Some(p) = (d.scan)(&f) else { continue };
                let Some(key) = rel_key(&root, &f) else {
                    continue;
                };
                out.push(summary_from_scan(d, p, key, &f));
            }
        }
        Layout::Antigravity => {
            let mut index = antigravity_summary_index(&root);
            if d.source == "antigravity-backup" {
                if let Some(primary) = antigravity_root().filter(|path| path.is_dir()) {
                    for (id, value) in antigravity_summary_index(&primary) {
                        index.entry(id).or_insert(value);
                    }
                }
            }
            let conv_dir = root.join("conversations");
            let mut ids = HashSet::new();
            if let Ok(entries) = fs::read_dir(&conv_dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    if path.extension().and_then(|s| s.to_str()) != Some("pb") {
                        continue;
                    }
                    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let Some(id) = extract_uuid(stem) else {
                        continue;
                    };
                    let metadata = match e.metadata() {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    ids.insert(id.clone());
                    let (title, cwd) = index.get(&id).cloned().unwrap_or_else(|| {
                        ("Antigravity 会话".to_string(), "Antigravity".to_string())
                    });
                    let cwd = if cwd.trim().is_empty() {
                        "Antigravity".to_string()
                    } else {
                        cwd
                    };
                    out.push(SessionSummary {
                        session_id: format!("conversations/{id}.pb"),
                        title,
                        session_type: d.source.to_string(),
                        workspace_directory: cwd.clone(),
                        workspace_hash: format!("{}{cwd}", d.prefix),
                        message_count: 0,
                        file_size: metadata.len(),
                        created_at: metadata_secs(&path, true),
                        modified_at: metadata_secs(&path, false),
                        source: d.source.to_string(),
                    });
                }
            }
            if d.source == "antigravity" {
                let index_path = root.join("agyhub_summaries_proto.pb");
                for (id, (title, cwd)) in index {
                    if ids.contains(&id) {
                        continue;
                    }
                    let cwd = if cwd.trim().is_empty() {
                        "Antigravity 历史".to_string()
                    } else {
                        cwd
                    };
                    out.push(SessionSummary {
                        session_id: virtual_session_id("summary", &id),
                        title,
                        session_type: d.source.to_string(),
                        workspace_directory: cwd.clone(),
                        workspace_hash: format!("{}{cwd}", d.prefix),
                        message_count: 0,
                        file_size: 0,
                        created_at: metadata_secs(&index_path, true),
                        modified_at: metadata_secs(&index_path, false),
                        source: d.source.to_string(),
                    });
                }
            }
        }
        Layout::AntigravityIde => {
            out.extend(collect_antigravity_ide_source(d, &root));
        }
        Layout::Gemini => {
            let mut files = Vec::new();
            collect_files(&root, "jsonl", 4, &mut files);
            for file in files {
                let Some(parsed) = (d.scan)(&file) else {
                    continue;
                };
                let Some(key) = rel_key(&root, &file) else {
                    continue;
                };
                out.push(summary_from_scan(d, parsed, key, &file));
            }
        }
    }
    out
}

fn antigravity_summary_id(summary: &SessionSummary) -> Option<String> {
    if !matches!(
        summary.source.as_str(),
        "antigravity" | "antigravity-backup"
    ) {
        return None;
    }
    virtual_session_key(&summary.session_id, "summary")
        .and_then(extract_uuid)
        .or_else(|| {
            Path::new(&summary.session_id)
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(extract_uuid)
        })
}

fn antigravity_summary_priority(summary: &SessionSummary) -> u8 {
    match (
        summary.source.as_str(),
        is_virtual_session(&summary.session_id),
    ) {
        ("antigravity", false) => 3,
        ("antigravity-backup", false) => 2,
        _ => 1,
    }
}

fn dedupe_external_sessions(summaries: Vec<SessionSummary>) -> Vec<SessionSummary> {
    let mut out: Vec<SessionSummary> = Vec::with_capacity(summaries.len());
    let mut antigravity_ids: HashMap<String, usize> = HashMap::new();
    for summary in summaries {
        let Some(id) = antigravity_summary_id(&summary) else {
            out.push(summary);
            continue;
        };
        if let Some(index) = antigravity_ids.get(&id).copied() {
            if antigravity_summary_priority(&summary) > antigravity_summary_priority(&out[index]) {
                out[index] = summary;
            }
        } else {
            antigravity_ids.insert(id, out.len());
            out.push(summary);
        }
    }
    out
}

fn rel_key(root: &Path, file: &Path) -> Option<String> {
    file.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().replace('\\', "/"))
}

fn summary_from_scan(
    d: &SourceDef,
    p: ParsedSummary,
    session_id: String,
    file: &Path,
) -> SessionSummary {
    SessionSummary {
        session_id,
        title: p.title,
        session_type: d.source.to_string(),
        workspace_directory: p.cwd.clone(),
        workspace_hash: format!("{}{}", d.prefix, p.cwd),
        message_count: p.message_count,
        file_size: fs::metadata(file).map(|m| m.len()).unwrap_or(0),
        created_at: p.created.or_else(|| metadata_secs(file, true)),
        modified_at: p.updated.or_else(|| metadata_secs(file, false)),
        source: d.source.to_string(),
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    fn summary(source: &str, session_id: String) -> SessionSummary {
        SessionSummary {
            session_id,
            title: source.to_string(),
            session_type: source.to_string(),
            workspace_directory: "workspace".to_string(),
            workspace_hash: format!("{source}:workspace"),
            message_count: 0,
            file_size: 0,
            created_at: None,
            modified_at: None,
            source: source.to_string(),
        }
    }

    #[test]
    fn prefers_antigravity_body_over_summary_index() {
        let id = "11111111-1111-4111-8111-111111111111";
        let sessions = dedupe_external_sessions(vec![
            summary("antigravity", virtual_session_id("summary", id)),
            summary("antigravity-backup", format!("conversations/{id}.pb")),
        ]);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source, "antigravity-backup");
    }
}

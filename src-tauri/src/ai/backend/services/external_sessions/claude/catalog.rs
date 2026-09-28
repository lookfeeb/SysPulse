use super::*;

pub(super) fn collect_claude_source(
    d: &SourceDef,
    root: &Path,
    depth: usize,
) -> Vec<SessionSummary> {
    let mut files = Vec::new();
    collect_files(root, "jsonl", depth, &mut files);
    files.sort();
    let live_paths: HashSet<_> = files.iter().map(|path| file_cache_key(path)).collect();
    if let Ok(mut cache) = claude_summary_cache().lock() {
        cache.retain(|path, _| live_paths.contains(path));
    }
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

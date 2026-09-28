use super::*;

pub(super) fn codex_meta_title(meta: &CodexSessionMeta) -> String {
    let value = if !meta.title.trim().is_empty() {
        meta.title.as_str()
    } else if !meta.first_user_message.trim().is_empty() {
        meta.first_user_message.as_str()
    } else {
        meta.history
            .first()
            .map(|(_, text)| text.as_str())
            .unwrap_or_default()
    };
    title_from_user_text(value)
}

pub(super) fn merge_codex_summary_metadata(
    d: &SourceDef,
    summary: &mut SessionSummary,
    meta: CodexSessionMeta,
) {
    if !meta.title.trim().is_empty() {
        summary.title = codex_meta_title(&meta);
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
    summary.message_count = summary.message_count.max(meta.history.len());
}

pub(super) fn append_codex_file_summary(
    d: &SourceDef,
    file: &Path,
    session_id: String,
    metadata: &mut HashMap<String, CodexSessionMeta>,
    seen_ids: &mut HashSet<String>,
    out: &mut Vec<SessionSummary>,
) {
    let Some(parsed) = scan_codex_summary(file) else {
        return;
    };
    let stable_id = parsed.stable_id.clone();
    if stable_id
        .as_ref()
        .is_some_and(|id| !seen_ids.insert(id.clone()))
    {
        return;
    }
    let mut summary = summary_from_scan(d, parsed, session_id, file);
    if let Some(meta) = stable_id.as_ref().and_then(|id| metadata.remove(id)) {
        merge_codex_summary_metadata(d, &mut summary, meta);
    }
    if summary.workspace_directory.trim().is_empty() {
        let fallback = if virtual_session_key(&summary.session_id, "archive").is_some() {
            "Codex 归档"
        } else {
            "Codex 历史"
        };
        summary.workspace_directory = fallback.to_string();
        summary.workspace_hash = format!("{}{fallback}", d.prefix);
    }
    out.push(summary);
}

pub(super) fn codex_archive_root(root: &Path) -> Option<PathBuf> {
    root.parent().map(|home| home.join("archived_sessions"))
}

pub(super) fn codex_archive_session_id(root: &Path, file: &Path) -> Option<String> {
    let key = rel_key(root, file)?;
    Some(virtual_session_id("archive", &urlencoding::encode(&key)))
}

pub(super) fn codex_archive_session_path(root: &Path, session_id: &str) -> Option<PathBuf> {
    let encoded = virtual_session_key(session_id, "archive")?;
    let relative = urlencoding::decode(encoded).ok()?.into_owned();
    let archive_root = codex_archive_root(root)?;
    safe_path(&archive_root, &relative)
}

pub(super) fn collect_codex_source(
    d: &SourceDef,
    root: &Path,
    depth: usize,
) -> Vec<SessionSummary> {
    let mut files = Vec::new();
    collect_files(root, "jsonl", depth, &mut files);
    files.sort();
    let archive_root = codex_archive_root(root).filter(|path| path.is_dir());
    let mut archived_files = Vec::new();
    if let Some(archive_root) = archive_root.as_ref() {
        collect_files(archive_root, "jsonl", depth, &mut archived_files);
        archived_files.sort();
    }
    let live_paths = files
        .iter()
        .chain(archived_files.iter())
        .map(|path| file_cache_key(path))
        .collect::<HashSet<_>>();
    prune_codex_summary_cache(&live_paths);

    let mut metadata = codex_metadata_result(root).unwrap_or_default();
    let mut seen_ids = HashSet::new();
    let mut out = Vec::new();

    for file in files {
        let Some(key) = rel_key(root, &file) else {
            continue;
        };
        append_codex_file_summary(d, &file, key, &mut metadata, &mut seen_ids, &mut out);
    }

    if let Some(archive_root) = archive_root.as_ref() {
        for file in archived_files {
            let Some(session_id) = codex_archive_session_id(archive_root, &file) else {
                continue;
            };
            append_codex_file_summary(d, &file, session_id, &mut metadata, &mut seen_ids, &mut out);
        }
    }

    for meta in metadata.into_values() {
        if seen_ids.contains(&meta.id) {
            continue;
        }
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史".to_string()
        } else {
            meta.cwd.clone()
        };
        out.push(SessionSummary {
            session_id: virtual_session_id("index", &meta.id),
            title: codex_meta_title(&meta),
            session_type: d.source.to_string(),
            workspace_directory: cwd.clone(),
            workspace_hash: format!("{}{cwd}", d.prefix),
            message_count: meta
                .history
                .len()
                .max(usize::from(!meta.first_user_message.trim().is_empty())),
            file_size: 0,
            created_at: meta.created,
            modified_at: meta.updated,
            source: d.source.to_string(),
        });
    }
    out
}

use super::*;

#[derive(Clone, Default)]
pub(super) struct ClaudeSessionMeta {
    pub(super) id: String,
    pub(super) cwd: String,
    pub(super) title: String,
    pub(super) created: Option<i64>,
    pub(super) updated: Option<i64>,
    pub(super) user_messages: Vec<(i64, String)>,
}

pub(super) fn claude_history_text(value: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(display) = value.get("display").and_then(|value| value.as_str()) {
        if !display.trim().is_empty() {
            parts.push(display.trim().to_string());
        }
    }
    if let Some(pasted) = value
        .get("pastedContents")
        .and_then(|value| value.as_object())
    {
        for item in pasted.values() {
            let Some(content) = item.get("content").and_then(|value| value.as_str()) else {
                continue;
            };
            if !content.trim().is_empty() {
                parts.push(format!("[粘贴内容]\n{}", content.trim()));
            }
        }
    }
    parts.join("\n\n")
}

pub(super) fn claude_config_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(config_dir) = root.parent() {
        paths.push(config_dir.join(".claude.json"));
        if config_dir.file_name().and_then(|name| name.to_str()) == Some(".claude") {
            if let Some(parent) = config_dir.parent() {
                paths.push(parent.join(".claude.json"));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn claude_scan_config_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = claude_config_paths(root);
    let Some(config_dir) = root.parent() else {
        return paths;
    };
    let backup_dir = config_dir.join("backups");
    if let Ok(entries) = fs::read_dir(backup_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".claude.json.backup."))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn claude_metadata_result(
    root: &Path,
) -> anyhow::Result<HashMap<String, ClaudeSessionMeta>> {
    let claude_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Claude 数据目录"))?;
    let mut out: HashMap<String, ClaudeSessionMeta> = HashMap::new();
    let history_path = claude_home.join("history.jsonl");
    if let Ok(file) = fs::File::open(history_path) {
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(id) = value
                .get("sessionId")
                .or_else(|| value.get("session_id"))
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
            else {
                continue;
            };
            let timestamp = value
                .get("timestamp")
                .and_then(|value| value.as_i64())
                .map(epoch_secs)
                .unwrap_or_default();
            let text = claude_history_text(&value);
            let entry = out.entry(id.clone()).or_insert_with(|| ClaudeSessionMeta {
                id,
                ..Default::default()
            });
            if let Some(project) = value.get("project").and_then(|value| value.as_str()) {
                if !project.trim().is_empty() {
                    entry.cwd = project.to_string();
                }
            }
            if !text.trim().is_empty() {
                if entry.title.is_empty() {
                    entry.title = value
                        .get("display")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or(&text)
                        .to_string();
                }
                entry.user_messages.push((timestamp, text));
            }
            if timestamp > 0 {
                entry.created = [entry.created, Some(timestamp)].into_iter().flatten().min();
                entry.updated = [entry.updated, Some(timestamp)].into_iter().flatten().max();
            }
        }
    }

    for config_path in claude_scan_config_paths(root) {
        let Ok(content) = fs::read_to_string(config_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let Some(projects) = value.get("projects").and_then(|value| value.as_object()) else {
            continue;
        };
        for (project, project_meta) in projects {
            let Some(id) = project_meta
                .get("lastSessionId")
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
            else {
                continue;
            };
            let entry = out.entry(id.clone()).or_insert_with(|| ClaudeSessionMeta {
                id,
                ..Default::default()
            });
            if entry.cwd.is_empty() {
                entry.cwd = project.to_string();
            }
            if let Some(title) = project_meta
                .get("lastSessionFirstPrompt")
                .and_then(|value| value.as_str())
            {
                if entry.title.is_empty() && !title.trim().is_empty() {
                    entry.title = title.to_string();
                }
            }
            if let Some(updated) = project_meta
                .get("lastSessionModified")
                .and_then(|value| value.as_i64())
                .map(epoch_secs)
            {
                entry.updated = [entry.updated, Some(updated)].into_iter().flatten().max();
            }
        }
    }

    for entry in out.values_mut() {
        entry.user_messages.sort_by_key(|(timestamp, _)| *timestamp);
        entry.user_messages.dedup();
    }
    Ok(out)
}

pub(super) fn claude_meta_title(meta: &ClaudeSessionMeta) -> String {
    let text = if !meta.title.trim().is_empty() {
        meta.title.as_str()
    } else {
        meta.user_messages
            .first()
            .map(|(_, text)| text.as_str())
            .unwrap_or_default()
    };
    title_from_user_text(text)
}

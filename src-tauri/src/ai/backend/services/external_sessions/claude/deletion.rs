use super::*;

pub(super) fn claude_session_id_from_file(path: &Path) -> anyhow::Result<Option<String>> {
    let file = fs::File::open(path)?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(id) = value
            .get("sessionId")
            .or_else(|| value.get("session_id"))
            .and_then(|value| value.as_str())
            .and_then(extract_uuid)
        {
            return Ok(Some(id));
        }
    }
    Ok(path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(extract_uuid))
}

pub(super) fn rewrite_claude_history_without_ids(
    path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Claude history 路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let content = fs::read_to_string(path)?;
    let mut filtered = String::with_capacity(content.len());
    let mut changed = false;
    for line in content.split_inclusive('\n') {
        let json = line.trim_end_matches(['\r', '\n']);
        let remove = serde_json::from_str::<serde_json::Value>(json)
            .ok()
            .and_then(|value| {
                value
                    .get("sessionId")
                    .or_else(|| value.get("session_id"))
                    .and_then(|value| value.as_str())
                    .and_then(extract_uuid)
            })
            .is_some_and(|value| ids.contains(&value));
        if remove {
            changed = true;
        } else {
            filtered.push_str(line);
        }
    }
    if changed {
        crate::ai::backend::utils::fs::atomic_write(path, &filtered, "Claude 会话索引")
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

pub(super) fn rewrite_claude_config_without_ids(
    path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Claude 配置路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let content = fs::read_to_string(path)?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| anyhow::anyhow!("解析 Claude 配置失败 {}: {error}", path.display()))?;
    let Some(projects) = value
        .get_mut("projects")
        .and_then(|value| value.as_object_mut())
    else {
        return Ok(());
    };
    let mut changed = false;
    for project in projects.values_mut() {
        let Some(project) = project.as_object_mut() else {
            continue;
        };
        let matches = |key: &str| {
            project
                .get(key)
                .and_then(|value| value.as_str())
                .and_then(extract_uuid)
                .is_some_and(|value| ids.contains(&value))
        };
        let last_matches = matches("lastSessionId");
        let hint_matches = matches("lastHintSessionId");
        if last_matches {
            for key in [
                "lastSessionId",
                "lastSessionFirstPrompt",
                "lastSessionModified",
            ] {
                changed |= project.remove(key).is_some();
            }
        }
        if hint_matches {
            changed |= project.remove("lastHintSessionId").is_some();
        }
    }
    if !changed {
        return Ok(());
    }
    let mut output = serde_json::to_vec_pretty(&value)?;
    output.push(b'\n');
    crate::ai::backend::utils::fs::atomic_write_bytes(path, &output, "Claude 配置")
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

pub(super) fn delete_claude_session(
    root: &Path,
    path: Option<&Path>,
    session_id: &str,
) -> anyhow::Result<()> {
    let id = virtual_session_key(session_id, "history")
        .and_then(extract_uuid)
        .or_else(|| path.and_then(|path| claude_session_id_from_file(path).ok().flatten()))
        .ok_or_else(|| anyhow::anyhow!("无法识别 Claude 会话 ID，未执行不完整删除"))?;
    delete_claude_targets(
        root,
        &HashSet::from([id]),
        &path.into_iter().map(Path::to_path_buf).collect::<Vec<_>>(),
    )
}

pub(super) fn delete_claude_targets(
    root: &Path,
    ids: &HashSet<String>,
    paths: &[PathBuf],
) -> anyhow::Result<()> {
    let claude_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Claude 数据目录"))?;
    rewrite_claude_history_without_ids(&claude_home.join("history.jsonl"), ids)?;
    for config in claude_scan_config_paths(root) {
        rewrite_claude_config_without_ids(&config, ids)?;
    }
    // 索引必须清理成功才删除正文，以便失败后仍能重试。
    if claude_metadata_result(root)?
        .keys()
        .any(|id| ids.contains(id))
    {
        return Err(anyhow::anyhow!(
            "Claude 会话索引仍有残留，已保留正文以便重试"
        ));
    }
    let mut errors = Vec::new();
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(errors.join("；")))
    }
}

pub(super) fn delete_claude_workspace(
    d: &SourceDef,
    root: &Path,
    hash: &str,
) -> anyhow::Result<()> {
    let mut ids = HashSet::new();
    let mut paths = Vec::new();
    for summary in collect_claude_source(d, root, 4)
        .into_iter()
        .filter(|s| s.workspace_hash == hash)
    {
        if let Some(id) = virtual_session_key(&summary.session_id, "history").and_then(extract_uuid)
        {
            ids.insert(id);
        } else {
            let path = safe_path(root, &summary.session_id)
                .ok_or_else(|| anyhow::anyhow!("Claude 会话路径已变化，请刷新重试"))?;
            let id = claude_session_id_from_file(&path)?
                .ok_or_else(|| anyhow::anyhow!("无法识别 Claude 会话 ID"))?;
            ids.insert(id);
            paths.push(path);
        }
    }
    delete_claude_targets(root, &ids, &paths)
}

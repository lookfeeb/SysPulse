use super::*;

pub(super) fn remove_antigravity_named_artifacts(root: &Path, id: &str, errors: &mut Vec<String>) {
    let canonical_root = match root.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!("校验 {} 失败: {error}", root.display()));
            return;
        }
    };
    remove_antigravity_named_artifacts_in(root, &canonical_root, id, errors);
}

pub(super) fn remove_antigravity_named_artifacts_in(
    dir: &Path,
    canonical_root: &Path,
    id: &str,
    errors: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("扫描 {} 失败: {error}", dir.display()));
            return;
        }
    };
    let mut entries = match entries
        .map(|entry| entry.map_err(|error| format!("读取 {} 目录项失败: {error}", dir.display())))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    entries.sort_by_key(|entry| {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == format!("{id}.db") || name == format!("{id}.pb") {
            0
        } else if name == format!("{id}.db-wal") || name == format!("{id}.db-shm") {
            2
        } else {
            1
        }
    });
    let mut main_db_failed = false;
    for entry in entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("读取 {} 类型失败: {error}", path.display()));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let matches = name == id
            || name.starts_with(&format!("{id}."))
            || name.starts_with(&format!("{id}-"));
        if matches {
            if main_db_failed && (name == format!("{id}.db-wal") || name == format!("{id}.db-shm"))
            {
                errors.push(format!(
                    "主数据库删除失败，已保留 SQLite 伴生文件: {}",
                    path.display()
                ));
                continue;
            }
            if file_type.is_symlink() {
                errors.push(format!(
                    "拒绝删除符号链接形式的会话数据: {}",
                    path.display()
                ));
                continue;
            }
            let canonical_path = match path.canonicalize() {
                Ok(path) if path.starts_with(canonical_root) && path != canonical_root => path,
                Ok(_) => {
                    errors.push(format!(
                        "拒绝删除 Antigravity 数据目录之外的路径: {}",
                        path.display()
                    ));
                    continue;
                }
                Err(error) => {
                    errors.push(format!("校验 {} 失败: {error}", path.display()));
                    continue;
                }
            };
            let result = if file_type.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            if let Err(error) = result {
                if name == format!("{id}.db") {
                    main_db_failed = true;
                }
                errors.push(format!("删除 {} 失败: {error}", canonical_path.display()));
            }
            continue;
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            let inside_root = path
                .canonicalize()
                .is_ok_and(|path| path.starts_with(canonical_root));
            if inside_root {
                remove_antigravity_named_artifacts_in(&path, canonical_root, id, errors);
            } else {
                errors.push(format!(
                    "拒绝扫描 Antigravity 数据目录之外的路径: {}",
                    path.display()
                ));
            }
        }
    }
}

pub(super) fn antigravity_named_artifacts_remain(
    root: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<bool> {
    let canonical_root = root.canonicalize()?;
    antigravity_named_artifacts_remain_in(root, &canonical_root, ids)
}

pub(super) fn antigravity_named_artifacts_remain_in(
    dir: &Path,
    canonical_root: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if ids.iter().any(|id| {
            name == *id
                || name.starts_with(&format!("{id}."))
                || name.starts_with(&format!("{id}-"))
        }) {
            return Ok(true);
        }
        if file_type.is_dir() && !file_type.is_symlink() {
            let canonical = path.canonicalize()?;
            if !canonical.starts_with(canonical_root) {
                return Err(anyhow::anyhow!(
                    "拒绝扫描 Antigravity 数据目录之外的路径: {}",
                    path.display()
                ));
            }
            if antigravity_named_artifacts_remain_in(&path, canonical_root, ids)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn remove_antigravity_proto_index(
    path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(raw) = read_antigravity_index(path)? else {
        return Ok(());
    };
    let (filtered, removed) = remove_protobuf_map_entries(&raw, ids)?;
    if removed == 0 {
        return Ok(());
    }
    crate::ai::backend::utils::fs::atomic_write_bytes(path, &filtered, "Antigravity 会话索引")
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

pub(super) fn remove_antigravity_ide_state_entries(ids: &HashSet<String>) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(path) = antigravity_ide_state_db() else {
        return Ok(());
    };
    match fs::metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    remove_antigravity_ide_state_entries_at(&path, ids)
}

pub(super) fn remove_antigravity_ide_state_entries_at(
    path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let trajectory_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let artifact_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let filtered_trajectory = trajectory_value
        .map(|value| {
            let decoded = general_purpose::STANDARD
                .decode(value)
                .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 轨迹索引失败: {error}"))?;
            remove_protobuf_map_entries(&decoded, ids)
        })
        .transpose()?;
    let filtered_artifact = artifact_value
        .map(|value| {
            let decoded = general_purpose::STANDARD
                .decode(value)
                .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 产物索引失败: {error}"))?;
            remove_protobuf_string_map_entries(&decoded, ids)
        })
        .transpose()?;
    let tx = conn.transaction()?;
    if let Some((filtered, removed)) = filtered_trajectory {
        if removed > 0 {
            tx.execute(
                "update ItemTable set value = ?1 where key = ?2",
                rusqlite::params![
                    general_purpose::STANDARD.encode(filtered),
                    "antigravityUnifiedStateSync.trajectorySummaries"
                ],
            )?;
        }
    }
    if let Some((filtered, removed)) = filtered_artifact {
        if removed > 0 {
            tx.execute(
                "update ItemTable set value = ?1 where key = ?2",
                rusqlite::params![
                    general_purpose::STANDARD.encode(filtered),
                    "antigravityUnifiedStateSync.artifactReview"
                ],
            )?;
        }
    }
    for id in ids {
        tx.execute(
            "delete from ItemTable where key like ?1",
            [format!("%{id}%")],
        )?;
    }
    tx.commit()?;
    let _ = conn.execute_batch("pragma wal_checkpoint(truncate)");
    Ok(())
}

pub(super) fn antigravity_ide_state_entries_remain(ids: &HashSet<String>) -> anyhow::Result<bool> {
    let Some(path) = antigravity_ide_state_db() else {
        return Ok(false);
    };
    match fs::metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let trajectory_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(value) = trajectory_value {
        let decoded = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 轨迹索引失败: {error}"))?;
        if protobuf_map_entries_result(&decoded)?
            .iter()
            .any(|(_, id)| ids.contains(id))
        {
            return Ok(true);
        }
    }
    let artifact_value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(value) = artifact_value {
        let decoded = general_purpose::STANDARD
            .decode(value)
            .map_err(|error| anyhow::anyhow!("解析 Antigravity IDE 产物索引失败: {error}"))?;
        if protobuf_string_map_entries_result(&decoded)?
            .iter()
            .any(|(path, _)| extract_uuid(path).is_some_and(|id| ids.contains(&id)))
        {
            return Ok(true);
        }
    }
    for id in ids {
        let remains = conn.query_row(
            "select exists(select 1 from ItemTable where key like ?1)",
            [format!("%{id}%")],
            |row| row.get::<_, bool>(0),
        )?;
        if remains {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn antigravity_id_from_session_id(session_id: &str) -> Option<String> {
    session_id
        .strip_prefix("brain/")
        .and_then(extract_uuid)
        .or_else(|| {
            Path::new(session_id)
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(extract_uuid)
        })
}

pub(super) fn antigravity_artifact_workspace_result(dir: &Path) -> anyhow::Result<Option<String>> {
    for name in ["task.md", "implementation_plan.md", "walkthrough.md"] {
        let path = dir.join(name);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for line in BufReader::new(file).lines() {
            let line = line?;
            if let Some(cwd) = clean_file_uri_at(&line) {
                return Ok(Some(cwd));
            }
        }
    }
    Ok(None)
}

pub(super) fn antigravity_conversation_db_workspace_result(
    root: &Path,
    id: &str,
) -> anyhow::Result<Option<String>> {
    let path = root.join("conversations").join(format!("{id}.db"));
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Antigravity 会话数据库路径不是文件: {}",
                path.display()
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    Ok(Some(antigravity_ide_db_stats_result(&path)?.0).filter(|cwd| !cwd.is_empty()))
}

pub(super) fn optional_read_dir(path: &Path) -> anyhow::Result<Option<fs::ReadDir>> {
    match fs::read_dir(path) {
        Ok(entries) => Ok(Some(entries)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn antigravity_ids_for_workspace(
    d: &SourceDef,
    root: &Path,
    workspace: &str,
) -> anyhow::Result<HashSet<String>> {
    let index = if matches!(d.layout, Layout::AntigravityIde) {
        antigravity_ide_index_result()?
            .into_iter()
            .collect::<HashMap<_, _>>()
    } else {
        antigravity_summary_index_result(root)?
            .into_iter()
            .map(|(id, (_, cwd))| {
                (
                    id,
                    AntigravityIdeIndexEntry {
                        title: String::new(),
                        cwd,
                    },
                )
            })
            .collect::<HashMap<_, _>>()
    };
    antigravity_ids_for_workspace_with_index(d, root, workspace, &index)
}

pub(super) fn antigravity_ids_for_workspace_with_index(
    d: &SourceDef,
    root: &Path,
    workspace: &str,
    index: &HashMap<String, AntigravityIdeIndexEntry>,
) -> anyhow::Result<HashSet<String>> {
    let target = normalize_codex_workspace(workspace);
    let mut ids = HashSet::new();
    let default_workspace = if matches!(d.layout, Layout::AntigravityIde) {
        "Antigravity IDE"
    } else {
        "Antigravity"
    };
    for (id, entry) in index {
        let cwd = if entry.cwd.trim().is_empty() {
            default_workspace
        } else {
            &entry.cwd
        };
        if normalize_codex_workspace(cwd) == target {
            ids.insert(id.clone());
        }
    }

    let scan_root = root.join("conversations");
    if let Some(entries) = optional_read_dir(&scan_root)? {
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_db = name.ends_with(".db");
            let is_pb = name.ends_with(".pb");
            let is_db_sidecar = name.ends_with(".db-wal") || name.ends_with(".db-shm");
            if !is_db && !is_pb && !is_db_sidecar {
                continue;
            }
            let Some(id) = extract_uuid(&name) else {
                continue;
            };
            let indexed_cwd = index
                .get(&id)
                .map(|entry| entry.cwd.trim().to_string())
                .filter(|cwd| !cwd.is_empty());
            let db_path = if is_db {
                Some(path)
            } else if is_db_sidecar {
                let candidate = scan_root.join(format!("{id}.db"));
                match fs::metadata(&candidate) {
                    Ok(metadata) if metadata.is_file() => Some(candidate),
                    Ok(_) => {
                        return Err(anyhow::anyhow!(
                            "Antigravity 会话数据库路径不是文件: {}",
                            candidate.display()
                        ));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                }
            } else {
                None
            };
            let db_cwd = db_path
                .as_deref()
                .map(antigravity_ide_db_stats_result)
                .transpose()?
                .map(|(cwd, _)| cwd)
                .filter(|cwd| !cwd.is_empty());
            let cwd = indexed_cwd
                .or(db_cwd)
                .unwrap_or_else(|| default_workspace.to_string());
            if normalize_codex_workspace(&cwd) == target {
                ids.insert(id);
            }
        }
    }
    if matches!(d.layout, Layout::AntigravityIde) {
        let brain = root.join("brain");
        if let Some(entries) = optional_read_dir(&brain)? {
            for entry in entries {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if !file_type.is_dir() || file_type.is_symlink() {
                    continue;
                }
                let Some(id) = entry.file_name().to_str().and_then(extract_uuid) else {
                    continue;
                };
                let cwd = index
                    .get(&id)
                    .map(|entry| entry.cwd.trim().to_string())
                    .filter(|cwd| !cwd.is_empty())
                    .or(antigravity_artifact_workspace_result(&entry.path())?)
                    .or(antigravity_conversation_db_workspace_result(root, &id)?)
                    .unwrap_or_else(|| default_workspace.to_string());
                if normalize_codex_workspace(&cwd) == target {
                    ids.insert(id);
                }
            }
        }
    }
    Ok(ids)
}

pub(super) fn delete_antigravity_ids(
    d: &SourceDef,
    root: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut errors = Vec::new();
    if matches!(d.layout, Layout::AntigravityIde) {
        if let Err(error) = remove_antigravity_ide_state_entries(ids) {
            errors.push(format!("清理 Antigravity IDE 轨迹索引失败: {error}"));
        }
    } else {
        let index_path = root.join("agyhub_summaries_proto.pb");
        if let Err(error) = remove_antigravity_proto_index(&index_path, ids) {
            errors.push(format!("清理 Antigravity 摘要索引失败: {error}"));
        }
    }
    // 状态索引无法更新时保留会话正文，避免产生无法从界面再次定位的孤儿记录。
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(errors.join("；")));
    }
    let metadata_remains = if matches!(d.layout, Layout::AntigravityIde) {
        antigravity_ide_state_entries_remain(ids)?
    } else {
        antigravity_summary_index_result(root)?
            .keys()
            .any(|id| ids.contains(id))
    };
    if metadata_remains {
        return Err(anyhow::anyhow!(
            "Antigravity 会话索引仍有残留，已保留正文文件以便重试"
        ));
    }
    for id in ids {
        remove_antigravity_named_artifacts(root, id, &mut errors);
    }
    if !errors.is_empty() {
        return Err(anyhow::anyhow!(errors.join("；")));
    }
    if antigravity_named_artifacts_remain(root, ids)? {
        return Err(anyhow::anyhow!(
            "Antigravity 会话文件仍有残留，请关闭 Antigravity 后重试"
        ));
    }
    Ok(())
}

pub(super) fn delete_antigravity_session(
    d: &SourceDef,
    root: &Path,
    session_id: &str,
) -> anyhow::Result<()> {
    let id = antigravity_id_from_session_id(session_id)
        .ok_or_else(|| anyhow::anyhow!("无法识别 Antigravity 会话 ID"))?;
    delete_antigravity_ids(d, root, &HashSet::from([id]))
}

pub(super) fn delete_antigravity_workspace(
    d: &SourceDef,
    root: &Path,
    workspace: &str,
) -> anyhow::Result<()> {
    if normalize_codex_workspace(workspace).is_empty() {
        return Err(anyhow::anyhow!("Antigravity 工作区路径为空，已拒绝删除"));
    }
    let ids = antigravity_ids_for_workspace(d, root, workspace)?;
    delete_antigravity_ids(d, root, &ids)?;
    if !antigravity_ids_for_workspace(d, root, workspace)?.is_empty() {
        return Err(anyhow::anyhow!(
            "Antigravity 工作区仍有残留记录，请关闭 Antigravity 后重试"
        ));
    }
    Ok(())
}

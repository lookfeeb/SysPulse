use super::*;

pub(super) fn normalize_codex_workspace(value: &str) -> String {
    let value = value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| value.strip_prefix(r"\\?\").unwrap_or(value).to_string());
    value
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

pub(super) fn codex_session_identity(
    path: &Path,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let Ok(value) = serde_json::from_str::<CodexEnvelope<'_>>(&line) else {
            continue;
        };
        if value.kind.as_deref() != Some("session_meta") {
            continue;
        }
        let payload = value.payload.as_ref();
        let id = payload
            .and_then(|value| value.id.as_deref().or(value.session_id.as_deref()))
            .and_then(extract_uuid);
        let cwd = payload
            .and_then(|value| value.cwd.as_deref())
            .map(str::to_string);
        return Ok((id, cwd));
    }
    Ok((None, None))
}

pub(super) fn codex_id_from_filename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(extract_uuid)
}

pub(super) fn codex_id_from_path(path: &Path) -> anyhow::Result<Option<String>> {
    Ok(codex_session_identity(path)?
        .0
        .or_else(|| codex_id_from_filename(path)))
}

pub(super) fn codex_database_paths(
    codex_home: &Path,
    prefix: &str,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(codex_home)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".sqlite"))
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub(super) fn collect_codex_files_strict(
    dir: &Path,
    ext: &str,
    depth: usize,
    out: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_codex_files_strict(&path, ext, depth - 1, out)?;
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some(ext)
        {
            out.push(path);
        }
    }
    Ok(())
}

pub(super) fn sqlite_table_names(conn: &Connection) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "select name from sqlite_master where type = 'table' and name not like 'sqlite_%'",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn sqlite_table_columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let quoted = table.replace('"', "\"\"");
    let mut stmt = conn.prepare(&format!("pragma table_info(\"{quoted}\")"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn codex_state_rows(db_path: &Path) -> anyhow::Result<Vec<(String, String, String)>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    if !sqlite_table_names(&conn)?
        .iter()
        .any(|name| name == "threads")
    {
        return Ok(Vec::new());
    }
    let columns = sqlite_table_columns(&conn, "threads")?;
    if !["id", "rollout_path", "cwd"]
        .iter()
        .all(|required| columns.iter().any(|column| column == required))
    {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare("select id, rollout_path, cwd from threads")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(super) fn safe_codex_rollout_path(root: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string());
    if raw.trim().is_empty() {
        return None;
    }
    let raw_path = PathBuf::from(&raw);
    let codex_home = root.parent()?;
    let archive_root = codex_archive_root(root);
    let candidates = if raw_path.is_absolute() {
        vec![raw_path]
    } else {
        let mut values = vec![root.join(&raw_path), codex_home.join(&raw_path)];
        if let Some(archive_root) = archive_root.as_ref() {
            values.push(archive_root.join(&raw_path));
        }
        values
    };
    let mut trusted_roots = vec![root.canonicalize().ok()?];
    if let Some(archive_root) = archive_root {
        if let Ok(path) = archive_root.canonicalize() {
            trusted_roots.push(path);
        }
    }
    candidates.into_iter().find_map(|candidate| {
        if !candidate.is_file() {
            return None;
        }
        let canonical = candidate.canonicalize().ok()?;
        trusted_roots
            .iter()
            .any(|trusted| canonical.starts_with(trusted))
            .then_some(canonical)
    })
}

pub(super) fn rewrite_jsonl_without_ids(
    path: &Path,
    id_field: &str,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "Codex 索引路径不是文件: {}",
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
                    .get(id_field)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .is_some_and(|id| ids.contains(&id));
        if remove {
            changed = true;
        } else {
            filtered.push_str(line);
        }
    }

    if changed {
        crate::ai::backend::utils::fs::atomic_write(path, &filtered, "Codex 会话索引")
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

pub(super) fn delete_thread_rows_from_db(
    db_path: &Path,
    ids: &HashSet<String>,
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let mut table_columns = sqlite_table_names(&conn)?
        .into_iter()
        .map(|table| {
            let columns = sqlite_table_columns(&conn, &table)?;
            Ok((table, columns))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    // 先删关联表，再删 threads 主表；兼容未来新增的非级联外键。
    table_columns.sort_by(|left, right| {
        (left.0 == "threads", &left.0).cmp(&(right.0 == "threads", &right.0))
    });
    let mut sorted_ids = ids.iter().collect::<Vec<_>>();
    sorted_ids.sort();

    let transaction = conn.transaction()?;
    for (table, columns) in table_columns {
        let id_columns = columns
            .iter()
            .filter(|column| {
                column.as_str() == "thread_id"
                    || column.as_str() == "parent_thread_id"
                    || column.as_str() == "child_thread_id"
                    || (table == "threads" && column.as_str() == "id")
            })
            .cloned()
            .collect::<Vec<_>>();
        if id_columns.is_empty() {
            continue;
        }
        let table = table.replace('"', "\"\"");
        for column in id_columns {
            let column = column.replace('"', "\"\"");
            // 避免大型 logs 数据库按每个 ID 重复全表扫描。
            for chunk in sorted_ids.chunks(500) {
                let placeholders = std::iter::repeat("?")
                    .take(chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!("delete from \"{table}\" where \"{column}\" in ({placeholders})");
                transaction.execute(&sql, rusqlite::params_from_iter(chunk.iter().copied()))?;
            }
        }
    }
    transaction.commit()?;
    let _ = conn.execute_batch("pragma wal_checkpoint(truncate)");
    Ok(())
}

pub(super) fn cleanup_empty_codex_dirs(root: &Path, path: &Path) {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        let empty = fs::read_dir(dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty || fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

pub(super) fn delete_codex_targets(
    root: &Path,
    ids: &HashSet<String>,
    paths: &HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut metadata_errors = Vec::new();

    for prefix in ["state_", "logs_", "goals_", "memories_"] {
        match codex_database_paths_all(codex_home, prefix) {
            Ok(paths) => {
                for db_path in paths {
                    if let Err(error) = delete_thread_rows_from_db(&db_path, ids) {
                        metadata_errors.push(format!("清理 {} 失败: {error}", db_path.display()));
                    }
                }
            }
            Err(error) => metadata_errors.push(format!("枚举 Codex {prefix} 数据库失败: {error}")),
        }
    }

    match codex_catalog_database_paths_all(codex_home) {
        Ok(paths) => {
            for db_path in paths {
                if let Err(error) = delete_thread_rows_from_db(&db_path, ids) {
                    metadata_errors.push(format!("清理 {} 失败: {error}", db_path.display()));
                }
            }
        }
        Err(error) => metadata_errors.push(format!("枚举 Codex 本地会话目录数据库失败: {error}")),
    }

    for (name, field) in [
        ("session_index.jsonl", "id"),
        ("session_index.jsonl.bak", "id"),
        ("history.jsonl", "session_id"),
        ("history.jsonl.bak", "session_id"),
    ] {
        let path = codex_home.join(name);
        if let Err(error) = rewrite_jsonl_without_ids(&path, field, ids) {
            metadata_errors.push(format!("清理 {} 失败: {error}", path.display()));
        }
    }

    // 元数据清理失败时保留正文文件，确保用户关闭 Codex 后还能从 SysPulse 重试。
    if !metadata_errors.is_empty() {
        return Err(anyhow::anyhow!(metadata_errors.join("；")));
    }

    clear_codex_metadata_cache();
    let mut canonical_roots = Vec::new();
    if root.is_dir() {
        canonical_roots.push(root.canonicalize()?);
    }
    if let Some(archive_root) = codex_archive_root(root) {
        if archive_root.is_dir() {
            canonical_roots.push(archive_root.canonicalize()?);
        }
    }
    let mut file_errors = Vec::new();
    for path in paths {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                file_errors.push(format!("Codex 会话路径不是文件: {}", path.display()));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                file_errors.push(format!("读取 {} 失败: {error}", path.display()));
                continue;
            }
        }
        let (canonical, trusted_root) = match path.canonicalize() {
            Ok(path) if path.is_file() => {
                let trusted_root = canonical_roots
                    .iter()
                    .find(|root| path.starts_with(root))
                    .cloned();
                match trusted_root {
                    Some(root) => (path, root),
                    None => {
                        file_errors.push(format!(
                            "拒绝删除 Codex 会话目录之外的路径: {}",
                            path.display()
                        ));
                        continue;
                    }
                }
            }
            Ok(_) => {
                file_errors.push(format!("Codex 会话路径不是文件: {}", path.display()));
                continue;
            }
            Err(error) => {
                file_errors.push(format!("校验 {} 失败: {error}", path.display()));
                continue;
            }
        };
        match fs::remove_file(&canonical) {
            Ok(()) => cleanup_empty_codex_dirs(&trusted_root, &canonical),
            Err(error) => file_errors.push(format!("删除 {} 失败: {error}", path.display())),
        }
    }

    if file_errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(file_errors.join("；")))
    }
}

pub(super) fn delete_codex_virtual_session(root: &Path, session_id: &str) -> anyhow::Result<()> {
    if let Some(raw_id) = virtual_session_key(session_id, "index") {
        let id = extract_uuid(raw_id)
            .ok_or_else(|| anyhow::anyhow!("Codex 索引会话 ID 无效，未执行清理"))?;
        let ids = HashSet::from([id.clone()]);
        delete_codex_targets(root, &ids, &HashSet::new())?;
        if codex_metadata_result(root)?.contains_key(&id) {
            return Err(anyhow::anyhow!(
                "Codex 索引仍有残留记录，请关闭 Codex 后重试"
            ));
        }
        return Ok(());
    }

    if virtual_session_key(session_id, "archive").is_some() {
        let path = codex_archive_session_path(root, session_id)
            .ok_or_else(|| anyhow::anyhow!("Codex 归档会话路径无效或已不存在"))?;
        return delete_codex_session(root, &path);
    }

    Err(anyhow::anyhow!("不支持的 Codex 虚拟会话类型"))
}

pub(super) fn delete_codex_session(root: &Path, path: &Path) -> anyhow::Result<()> {
    let id = codex_id_from_path(path)?
        .ok_or_else(|| anyhow::anyhow!("无法识别 Codex 会话 ID，未执行不完整删除"))?;
    let ids = HashSet::from([id]);
    let paths = HashSet::from([path.to_path_buf()]);
    delete_codex_targets(root, &ids, &paths)
}

pub(super) fn delete_codex_workspace(root: &Path, workspace: &str) -> anyhow::Result<()> {
    let target = normalize_codex_workspace(workspace);
    if target.is_empty() {
        return Err(anyhow::anyhow!("Codex 工作区路径为空，已拒绝删除"));
    }
    let codex_home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    let mut unidentified_paths = Vec::new();

    let mut rollout_roots = vec![root.to_path_buf()];
    if let Some(archive_root) = codex_archive_root(root) {
        if archive_root.is_dir() {
            rollout_roots.push(archive_root);
        }
    }
    for rollout_root in &rollout_roots {
        let mut files = Vec::new();
        collect_codex_files_strict(rollout_root, "jsonl", 8, &mut files)?;
        for path in files {
            let (id, cwd) = codex_session_identity(&path)?;
            if cwd
                .as_deref()
                .is_some_and(|cwd| normalize_codex_workspace(cwd) == target)
            {
                match id.or_else(|| codex_id_from_filename(&path)) {
                    Some(id) => {
                        ids.insert(id);
                        paths.insert(path);
                    }
                    None => unidentified_paths.push(path),
                }
            }
        }
    }

    if !unidentified_paths.is_empty() {
        let paths = unidentified_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("、");
        return Err(anyhow::anyhow!(
            "以下 Codex 会话无法识别 ID，未执行不完整删除: {paths}"
        ));
    }

    for db_path in codex_database_paths_all(codex_home, "state_")? {
        for (id, rollout_path, cwd) in codex_state_rows(&db_path)? {
            if normalize_codex_workspace(&cwd) != target {
                continue;
            }
            ids.insert(id);
            if let Some(path) = safe_codex_rollout_path(root, &rollout_path) {
                paths.insert(path);
            }
        }
    }

    for meta in codex_metadata_result(root)?.into_values() {
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史"
        } else {
            meta.cwd.as_str()
        };
        if normalize_codex_workspace(cwd) != target {
            continue;
        }
        ids.insert(meta.id);
        if let Some(path) = safe_codex_rollout_path(root, &meta.rollout_path) {
            paths.insert(path);
        }
    }

    delete_codex_targets(root, &ids, &paths)?;

    let mut file_remains = false;
    for rollout_root in &rollout_roots {
        let mut remaining_files = Vec::new();
        collect_codex_files_strict(rollout_root, "jsonl", 8, &mut remaining_files)?;
        for path in &remaining_files {
            if codex_session_identity(path)?
                .1
                .as_deref()
                .is_some_and(|cwd| normalize_codex_workspace(cwd) == target)
            {
                file_remains = true;
                break;
            }
        }
        if file_remains {
            break;
        }
    }
    let metadata_remains = codex_metadata_result(root)?.into_values().any(|meta| {
        let cwd = if meta.cwd.trim().is_empty() {
            "Codex 历史"
        } else {
            meta.cwd.as_str()
        };
        normalize_codex_workspace(cwd) == target
    });
    if file_remains || metadata_remains {
        return Err(anyhow::anyhow!(
            "Codex 工作区仍有残留记录，请关闭 Codex 后重试"
        ));
    }
    Ok(())
}

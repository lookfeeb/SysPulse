use super::*;
use std::sync::Arc;

type Fingerprint = Vec<(PathBuf, Option<FileStamp>)>;
struct MetadataEntry {
    fingerprint: Fingerprint,
    data: Arc<HashMap<String, CodexSessionMeta>>,
}

fn metadata_cache() -> &'static Mutex<HashMap<PathBuf, MetadataEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, MetadataEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fingerprint(root: &Path) -> anyhow::Result<Fingerprint> {
    let home = root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无法定位 Codex 数据目录"))?;
    let mut paths = codex_database_paths_all(home, "state_")?;
    paths.extend(codex_catalog_database_paths_all(home)?);
    // WAL 提交不会立刻修改主数据库文件，必须一起纳入失效条件。
    let sidecars: Vec<_> = paths
        .iter()
        .flat_map(|path| {
            ["-wal", "-journal"].map(|suffix| {
                let mut value = path.as_os_str().to_os_string();
                value.push(suffix);
                PathBuf::from(value)
            })
        })
        .collect();
    paths.extend(sidecars);
    for name in [
        "session_index.jsonl",
        "session_index.jsonl.bak",
        "history.jsonl",
        "history.jsonl.bak",
        "config.toml",
    ] {
        paths.push(home.join(name));
    }
    paths.sort();
    paths.dedup();
    Ok(paths
        .into_iter()
        .map(|path| {
            let stamp = file_stamp(&path);
            (path, stamp)
        })
        .collect())
}

pub(super) fn clear_codex_metadata_cache() {
    if let Ok(mut cache) = metadata_cache().lock() {
        cache.clear();
    }
}

pub(super) fn codex_metadata_snapshot(
    root: &Path,
) -> anyhow::Result<Arc<HashMap<String, CodexSessionMeta>>> {
    let before = fingerprint(root)?;
    let key = file_cache_key(root);
    if let Ok(cache) = metadata_cache().lock() {
        if let Some(entry) = cache.get(&key).filter(|entry| entry.fingerprint == before) {
            return Ok(entry.data.clone());
        }
    }
    let (data, complete) = read_codex_metadata(root)?;
    let data = Arc::new(data);
    let bytes: usize = data
        .values()
        .map(|meta| {
            meta.id.len()
                + meta.rollout_path.len()
                + meta.cwd.len()
                + meta.title.len()
                + meta.first_user_message.len()
                + meta
                    .history
                    .iter()
                    .map(|(_, text)| text.len() + 32)
                    .sum::<usize>()
                + 256
        })
        .sum();
    if complete && bytes <= 16 * 1024 * 1024 && fingerprint(root)? == before {
        if let Ok(mut cache) = metadata_cache().lock() {
            // 只保留当前数据根，避免切换配置目录后积累凭据之外的历史文本。
            cache.clear();
            cache.insert(
                key,
                MetadataEntry {
                    fingerprint: before,
                    data: data.clone(),
                },
            );
        }
    }
    Ok(data)
}

pub(super) fn codex_metadata_for_id(
    root: &Path,
    id: &str,
) -> anyhow::Result<Option<CodexSessionMeta>> {
    Ok(codex_metadata_snapshot(root)?.get(id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_cache_observes_wal_commits_and_jsonl_updates() {
        let home =
            std::env::temp_dir().join(format!("syspulse-codex-meta-{}", uuid::Uuid::new_v4()));
        let root = home.join("sessions");
        fs::create_dir_all(&root).unwrap();
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE threads(id TEXT PRIMARY KEY, cwd TEXT, title TEXT, rollout_path TEXT, created_at INTEGER, updated_at INTEGER);").unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        db.execute(
            "INSERT INTO threads VALUES(?1,'C:/project','before','',1,1)",
            [id],
        )
        .unwrap();
        assert_eq!(
            codex_metadata_for_id(&root, id).unwrap().unwrap().title,
            "before"
        );
        db.execute("UPDATE threads SET title='after', updated_at=2", [])
            .unwrap();
        assert_eq!(
            codex_metadata_for_id(&root, id).unwrap().unwrap().title,
            "after"
        );
        fs::write(
            home.join("session_index.jsonl"),
            format!("{{\"id\":\"{id}\",\"thread_name\":\"index title\"}}\n"),
        )
        .unwrap();
        assert_eq!(
            codex_metadata_for_id(&root, id).unwrap().unwrap().title,
            "index title"
        );
        drop(db);
        fs::remove_dir_all(home).unwrap();
    }
}

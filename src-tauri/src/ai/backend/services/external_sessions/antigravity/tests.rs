use super::*;

fn no_test_root() -> Option<PathBuf> {
    None
}

fn no_test_summary(_: &Path) -> Option<ParsedSummary> {
    None
}

fn test_source(layout: Layout) -> SourceDef {
    SourceDef {
        prefix: "test:",
        source: "test",
        root: no_test_root,
        layout,
        parse: parse_antigravity,
        scan: no_test_summary,
    }
}

fn test_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn varint(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

fn map_entry(id: &str, text: &str) -> Vec<u8> {
    let mut payload = vec![0x0a, 36];
    payload.extend_from_slice(id.as_bytes());
    payload.extend_from_slice(&[0x12, text.len() as u8]);
    payload.extend_from_slice(text.as_bytes());
    let mut field = vec![0x0a];
    field.extend(varint(payload.len()));
    field.extend(payload);
    field
}

fn bytes_field(number: usize, payload: &[u8]) -> Vec<u8> {
    let mut field = varint((number << 3) | 2);
    field.extend(varint(payload.len()));
    field.extend_from_slice(payload);
    field
}

fn string_map_entry(key: &str, value: &[u8]) -> Vec<u8> {
    let mut entry = bytes_field(1, key.as_bytes());
    entry.extend(bytes_field(2, value));
    bytes_field(1, &entry)
}

#[test]
fn removes_only_matching_antigravity_summary_entries() {
    let keep = "11111111-1111-4111-8111-111111111111";
    let remove = "22222222-2222-4222-8222-222222222222";
    let mut bytes = map_entry(keep, "keep");
    bytes.extend(map_entry(remove, "remove"));
    let (filtered, count) =
        remove_protobuf_map_entries(&bytes, &HashSet::from([remove.to_string()])).unwrap();
    assert_eq!(count, 1);
    assert_eq!(protobuf_map_entries(&filtered).len(), 1);
    assert_eq!(protobuf_map_entries(&filtered)[0].1, keep);
}

#[test]
fn rejects_truncated_antigravity_index_instead_of_overwriting_it() {
    let result = remove_protobuf_map_entries(&[0x0a, 0xff], &HashSet::new());
    assert!(result.is_err());
}

#[test]
fn restores_antigravity_artifact_review_ids_and_titles() {
    let titled = "11111111-1111-4111-8111-111111111111";
    let untitled = "22222222-2222-4222-8222-222222222222";
    let metadata = bytes_field(1, "恢复的实施计划".as_bytes());
    let json = serde_json::to_vec(&serde_json::json!({
        "artifactMetadata": general_purpose::STANDARD.encode(metadata)
    }))
    .unwrap();
    let wrapped = bytes_field(1, &json);
    let mut bytes = string_map_entry(&format!("file:///tmp/brain/{titled}/task.md"), &wrapped);
    let empty_json = bytes_field(1, br#"{"comments":[]}"#);
    bytes.extend(string_map_entry(
        &format!("file:///tmp/brain/{untitled}/walkthrough.md"),
        &empty_json,
    ));

    let titles = antigravity_ide_artifact_titles(&bytes).unwrap();
    assert_eq!(titles.len(), 2);
    assert_eq!(
        titles.get(titled).map(String::as_str),
        Some("恢复的实施计划")
    );
    assert_eq!(titles.get(untitled).map(String::as_str), Some(""));
}

#[test]
fn keeps_antigravity_artifacts_when_index_cleanup_fails() {
    let root = test_dir("antigravity-index-failure");
    let conversations = root.join("conversations");
    fs::create_dir_all(&conversations).unwrap();
    let id = "22222222-2222-4222-8222-222222222222";
    let conversation = conversations.join(format!("{id}.pb"));
    fs::write(&conversation, b"conversation").unwrap();
    fs::write(root.join("agyhub_summaries_proto.pb"), [0x0a, 0xff]).unwrap();
    let source = SOURCES
        .iter()
        .find(|source| source.source == "antigravity")
        .unwrap();

    assert!(delete_antigravity_ids(source, &root, &HashSet::from([id.to_string()])).is_err());
    assert!(conversation.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removes_antigravity_named_artifacts_without_touching_root() {
    let root = test_dir("antigravity-artifacts");
    let conversations = root.join("conversations");
    let brain = root.join("brain");
    fs::create_dir_all(&conversations).unwrap();
    fs::create_dir_all(&brain).unwrap();
    let id = "22222222-2222-4222-8222-222222222222";
    for suffix in [".db", ".db-wal", ".db-shm", ".pb"] {
        fs::write(conversations.join(format!("{id}{suffix}")), b"data").unwrap();
    }
    fs::create_dir_all(brain.join(id)).unwrap();
    let keep = conversations.join("keep.db");
    fs::write(&keep, b"keep").unwrap();
    let mut errors = Vec::new();

    remove_antigravity_named_artifacts(&root, id, &mut errors);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(root.exists());
    assert!(keep.exists());
    assert!(!brain.join(id).exists());
    for suffix in [".db", ".db-wal", ".db-shm", ".pb"] {
        assert!(!conversations.join(format!("{id}{suffix}")).exists());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_empty_antigravity_workspace() {
    let root = test_dir("antigravity-empty-workspace");
    let source = SOURCES
        .iter()
        .find(|source| source.source == "antigravity")
        .unwrap();
    assert!(delete_antigravity_workspace(source, &root, "").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn removes_antigravity_trajectory_and_notification_state() {
    let dir = test_dir("antigravity-state");
    let db = dir.join("state.vscdb");
    let keep = "11111111-1111-4111-8111-111111111111";
    let remove = "22222222-2222-4222-8222-222222222222";
    let mut bytes = map_entry(keep, "keep");
    bytes.extend(map_entry(remove, "remove"));
    let artifact = string_map_entry(
        &format!("file:///tmp/brain/{remove}/task.md"),
        &bytes_field(1, br#"{}"#),
    );
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("create table ItemTable (key text primary key, value text);")
        .unwrap();
    conn.execute(
        "insert into ItemTable (key, value) values (?1, ?2)",
        rusqlite::params![
            "antigravityUnifiedStateSync.trajectorySummaries",
            general_purpose::STANDARD.encode(bytes)
        ],
    )
    .unwrap();
    conn.execute(
        "insert into ItemTable (key, value) values (?1, ?2)",
        rusqlite::params![
            "antigravityUnifiedStateSync.artifactReview",
            general_purpose::STANDARD.encode(artifact)
        ],
    )
    .unwrap();
    conn.execute(
        "insert into ItemTable (key, value) values (?1, '1')",
        [format!("antigravity.notification.{remove}")],
    )
    .unwrap();
    drop(conn);

    remove_antigravity_ide_state_entries_at(&db, &HashSet::from([remove.to_string()])).unwrap();
    let conn = Connection::open(&db).unwrap();
    let value = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.trajectorySummaries"],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    let decoded = general_purpose::STANDARD.decode(value).unwrap();
    let entries = protobuf_map_entries(&decoded);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1, keep);
    let artifact = conn
        .query_row(
            "select value from ItemTable where key = ?1",
            ["antigravityUnifiedStateSync.artifactReview"],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    let artifact = general_purpose::STANDARD.decode(artifact).unwrap();
    assert!(protobuf_string_map_entries_result(&artifact)
        .unwrap()
        .is_empty());
    assert_eq!(
        conn.query_row(
            "select count(*) from ItemTable where key like ?1",
            [format!("%{remove}%")],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn removes_antigravity_notification_without_trajectory_index() {
    let dir = test_dir("antigravity-notification-only");
    let db = dir.join("state.vscdb");
    let remove = "22222222-2222-4222-8222-222222222222";
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("create table ItemTable (key text primary key, value text);")
        .unwrap();
    conn.execute(
        "insert into ItemTable (key, value) values (?1, '1')",
        [format!("antigravity.notification.10{remove}")],
    )
    .unwrap();
    drop(conn);

    remove_antigravity_ide_state_entries_at(&db, &HashSet::from([remove.to_string()])).unwrap();
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("select count(*) from ItemTable", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reads_current_antigravity_sqlite_conversation_format() {
    let dir = test_dir("antigravity-db");
    let db = dir.join("11111111-1111-4111-8111-111111111111.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "create table trajectory_metadata_blob (id text primary key, data blob);\
             create table steps (idx integer primary key, step_payload blob);",
    )
    .unwrap();
    conn.execute(
        "insert into trajectory_metadata_blob (id, data) values ('main', ?1)",
        [b"meta file:///c:/Users/Test/Project".as_slice()],
    )
    .unwrap();
    conn.execute("insert into steps values (1, x'01')", [])
        .unwrap();
    conn.execute("insert into steps values (2, x'02')", [])
        .unwrap();
    drop(conn);

    let (cwd, count) = antigravity_ide_db_stats(&db);
    assert_eq!(cwd, "C:\\Users\\Test\\Project");
    assert_eq!(count, 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn finds_orphan_antigravity_ide_artifacts_in_default_workspace() {
    let root = test_dir("antigravity-orphans");
    let brain_id = "11111111-1111-4111-8111-111111111111";
    let sidecar_id = "22222222-2222-4222-8222-222222222222";
    let db_id = "33333333-3333-4333-8333-333333333333";
    fs::create_dir_all(root.join("brain").join(brain_id)).unwrap();
    fs::create_dir_all(root.join("brain").join(db_id)).unwrap();
    fs::create_dir_all(root.join("conversations")).unwrap();
    fs::write(
        root.join("conversations")
            .join(format!("{sidecar_id}.db-wal")),
        [],
    )
    .unwrap();
    let db = root.join("conversations").join(format!("{db_id}.db"));
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("create table trajectory_metadata_blob (id text, data blob);")
        .unwrap();
    conn.execute(
        "insert into trajectory_metadata_blob values ('main', ?1)",
        [b"file:///C:/Workspace/Target".as_slice()],
    )
    .unwrap();
    drop(conn);

    let ids = antigravity_ids_for_workspace_with_index(
        &test_source(Layout::AntigravityIde),
        &root,
        "Antigravity IDE",
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        ids,
        HashSet::from([brain_id.to_string(), sidecar_id.to_string()])
    );
    let target_ids = antigravity_ids_for_workspace_with_index(
        &test_source(Layout::AntigravityIde),
        &root,
        "C:\\Workspace\\Target",
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(target_ids, HashSet::from([db_id.to_string()]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deletes_legacy_workspace_files_and_orphan_summary_entries() {
    let root = test_dir("antigravity-workspace");
    let conversations = root.join("conversations");
    fs::create_dir_all(&conversations).unwrap();
    let removed_id = "11111111-1111-4111-8111-111111111111";
    let orphan_id = "22222222-2222-4222-8222-222222222222";
    let keep_id = "33333333-3333-4333-8333-333333333333";
    fs::write(conversations.join(format!("{removed_id}.pb")), b"remove").unwrap();
    fs::write(conversations.join(format!("{keep_id}.pb")), b"keep").unwrap();
    let mut index = map_entry(removed_id, "file:///C:/Workspace/Target");
    index.extend(map_entry(orphan_id, "file:///C:/Workspace/Target"));
    index.extend(map_entry(keep_id, "file:///C:/Workspace/Other"));
    fs::write(root.join("agyhub_summaries_proto.pb"), index).unwrap();

    delete_antigravity_workspace(
        &test_source(Layout::Antigravity),
        &root,
        "C:\\Workspace\\Target",
    )
    .unwrap();

    assert!(!conversations.join(format!("{removed_id}.pb")).exists());
    assert!(conversations.join(format!("{keep_id}.pb")).exists());
    let remaining = fs::read(root.join("agyhub_summaries_proto.pb")).unwrap();
    let remaining_ids = protobuf_map_entries(&remaining)
        .into_iter()
        .map(|(_, id)| id)
        .collect::<HashSet<_>>();
    assert_eq!(remaining_ids, HashSet::from([keep_id.to_string()]));
    fs::remove_dir_all(root).unwrap();
}

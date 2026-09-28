use super::*;

fn no_test_root() -> Option<PathBuf> {
    None
}

fn test_source() -> SourceDef {
    SourceDef {
        prefix: "codex-test:",
        source: "codex",
        root: no_test_root,
        layout: Layout::File { depth: 8 },
        parse: parse_codex,
        scan: scan_codex_summary,
    }
}

fn test_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_rollout(path: &Path, id: &str, cwd: &str, message: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let lines = [
        serde_json::json!({
            "timestamp": "2026-08-17T01:00:00Z",
            "type": "session_meta",
            "payload": { "id": id, "cwd": cwd }
        })
        .to_string(),
        serde_json::json!({
            "timestamp": "2026-08-17T01:00:01Z",
            "type": "event_msg",
            "payload": { "type": "user_message", "message": message }
        })
        .to_string(),
    ];
    fs::write(path, lines.join("\n") + "\n").unwrap();
}

#[test]
fn typed_codex_parser_skips_large_irrelevant_payloads() {
    let huge = "x".repeat(256 * 1024);
    let content = format!(
        "{}\n{}\n{}\n",
        serde_json::json!({
            "timestamp": "2026-08-17T01:00:00Z",
            "type": "session_meta",
            "payload": { "id": "11111111-1111-4111-8111-111111111111", "cwd": "C:\\Work" }
        }),
        serde_json::json!({
            "timestamp": "2026-08-17T01:00:01Z",
            "type": "response_item",
            "payload": { "type": "custom_tool_call_output", "output": huge }
        }),
        serde_json::json!({
            "timestamp": "2026-08-17T01:00:02Z",
            "type": "event_msg",
            "payload": { "type": "user_message", "message": "保留这条用户消息" }
        })
    );
    let parsed = parse_codex(&content);
    assert_eq!(parsed.cwd, "C:\\Work");
    assert_eq!(parsed.blocks.len(), 1);
    assert_eq!(parsed.blocks[0].1, "保留这条用户消息");
}

#[test]
fn streams_codex_page_without_losing_workspace_offset() {
    let dir = test_dir("codex-page");
    let path = dir.join("rollout.jsonl");
    let lines = [
        serde_json::json!({
            "type": "session_meta",
            "payload": { "cwd": "C:\\Workspace\\Paged" }
        }),
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "user_message", "message": "第一条" }
        }),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "第二条" }]
            }
        }),
        serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "user_message", "message": "第三条" }
        }),
        serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "第四条" }]
            }
        }),
    ];
    fs::write(
        &path,
        lines
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    let page = load_codex_file_page(&test_source(), &path, "session", 2, 2, None, None).unwrap();
    assert_eq!(page.total_messages, 5);
    assert_eq!(page.title, "第一条");
    assert_eq!(page.history.len(), 2);
    assert_eq!(page.history[0].message.content[0].text, "第二条");
    assert_eq!(page.history[1].message.content[0].text, "第三条");

    let first = load_codex_file_page(&test_source(), &path, "session", 1, 2, None, None).unwrap();
    assert_eq!(first.history.len(), 2);
    assert_eq!(first.history[0].message.role, "system");
    assert_eq!(first.history[1].message.content[0].text, "第一条");

    let out_of_range =
        load_codex_file_page(&test_source(), &path, "session", 4, 2, None, None).unwrap();
    assert_eq!(out_of_range.total_messages, 5);
    assert!(out_of_range.history.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_summary_cache_invalidates_when_file_changes() {
    let dir = test_dir("codex-cache");
    let id = "11111111-1111-4111-8111-111111111111";
    let path = dir.join(format!("rollout-{id}.jsonl"));
    write_rollout(&path, id, "C:\\Work", "第一次标题");
    let first = scan_codex_summary(&path).unwrap();
    let cached = scan_codex_summary(&path).unwrap();
    assert_eq!(cached.title, first.title);

    write_rollout(
        &path,
        id,
        "C:\\Work",
        "第二次标题，长度不同以触发文件戳更新",
    );
    let changed = scan_codex_summary(&path).unwrap();
    assert_ne!(changed.title, first.title);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn scans_archived_rollouts_and_prefers_active_duplicate() {
    let home = test_dir("codex-archive");
    let root = home.join("sessions");
    let archive = home.join("archived_sessions");
    let active_id = "11111111-1111-4111-8111-111111111111";
    let archive_id = "22222222-2222-4222-8222-222222222222";
    write_rollout(
        &root.join(format!("rollout-{active_id}.jsonl")),
        active_id,
        "C:\\Active",
        "当前正文优先",
    );
    write_rollout(
        &archive.join(format!("rollout-{active_id}.jsonl")),
        active_id,
        "C:\\Archive",
        "重复归档不应出现",
    );
    write_rollout(
        &archive.join(format!("rollout-{archive_id}.jsonl")),
        archive_id,
        "C:\\Archive",
        "仅归档会话",
    );

    let source = test_source();
    let summaries = collect_codex_source(&source, &root, 8);
    assert_eq!(summaries.len(), 2);
    assert!(summaries.iter().any(|item| item.title == "当前正文优先"));
    assert!(!summaries
        .iter()
        .any(|item| item.title == "重复归档不应出现"));
    let archived = summaries
        .iter()
        .find(|item| item.title == "仅归档会话")
        .unwrap();
    assert!(archived.session_id.starts_with("@archive/"));
    let loaded = load_codex_virtual_session(
        &source,
        &root,
        &archived.session_id,
        &archived.workspace_hash,
    )
    .unwrap();
    assert!(loaded
        .history
        .iter()
        .any(|item| { item.message.content[0].text.contains("仅归档会话") }));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn restores_catalog_only_thread_and_history_backup() {
    let home = test_dir("codex-catalog");
    let root = home.join("sessions");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&sqlite_dir).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let db = sqlite_dir.join("codex-test.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "create table local_thread_catalog (\
                thread_id text, display_title text, cwd text,\
                source_created_at real, source_updated_at real, source_recency_at real\
             );",
    )
    .unwrap();
    conn.execute(
            "insert into local_thread_catalog values (?1, '旧目录标题', 'C:\\Catalog', 1700000000.0, 1700000100.0, 1700000200.0)",
            [id],
        )
        .unwrap();
    drop(conn);
    fs::write(
        home.join("history.jsonl.bak"),
        format!("{{\"session_id\":\"{id}\",\"ts\":1700000050,\"text\":\"备份历史输入\"}}\n"),
    )
    .unwrap();

    let source = test_source();
    let summaries = collect_codex_source(&source, &root, 8);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].session_id, virtual_session_id("index", id));
    assert_eq!(summaries[0].title, "旧目录标题");
    assert_eq!(summaries[0].workspace_directory, "C:\\Catalog");
    assert_eq!(summaries[0].message_count, 1);
    let loaded = load_codex_virtual_session(
        &source,
        &root,
        &summaries[0].session_id,
        &summaries[0].workspace_hash,
    )
    .unwrap();
    assert!(loaded
        .history
        .iter()
        .any(|item| { item.message.content[0].text.contains("备份历史输入") }));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn resolves_relative_sqlite_home_from_codex_home() {
    let home = test_dir("codex-sqlite-home");
    fs::write(home.join("config.toml"), "sqlite_home = 'relative-db'\n").unwrap();
    assert_eq!(
        codex_configured_sqlite_home(&home).unwrap(),
        home.join("relative-db")
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn rewrites_codex_indexes_by_session_id() {
    let dir = test_dir("codex-index");
    let path = dir.join("session_index.jsonl");
    fs::write(&path, "{\"id\":\"keep\"}\n{\"id\":\"remove\"}\nnot-json\n").unwrap();
    rewrite_jsonl_without_ids(&path, "id", &HashSet::from(["remove".to_string()])).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\"id\":\"keep\"}\nnot-json\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deletes_catalog_only_codex_index_session() {
    let home = test_dir("codex-index-delete");
    let root = home.join("sessions");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&sqlite_dir).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    fs::write(
        home.join("session_index.jsonl"),
        format!("{{\"id\":\"{id}\",\"thread_name\":\"残留索引\"}}\n"),
    )
    .unwrap();
    fs::write(
        home.join("history.jsonl"),
        format!("{{\"session_id\":\"{id}\",\"text\":\"残留输入\"}}\n"),
    )
    .unwrap();

    let db = sqlite_dir.join("codex-test.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "create table local_thread_catalog (thread_id text, display_title text, cwd text);",
    )
    .unwrap();
    conn.execute(
        "insert into local_thread_catalog values (?1, '残留索引', 'C:\\Catalog')",
        [id],
    )
    .unwrap();
    drop(conn);

    delete_codex_virtual_session(&root, &virtual_session_id("index", id)).unwrap();

    assert!(!fs::read_to_string(home.join("session_index.jsonl"))
        .unwrap()
        .contains(id));
    assert!(!fs::read_to_string(home.join("history.jsonl"))
        .unwrap()
        .contains(id));
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "select count(*) from local_thread_catalog where thread_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deletes_archived_codex_session_and_metadata() {
    let home = test_dir("codex-archive-delete");
    let root = home.join("sessions");
    let archive = home.join("archived_sessions");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let rollout = archive.join("2026").join(format!("rollout-{id}.jsonl"));
    write_rollout(&rollout, id, "C:\\Archive", "归档会话");
    fs::write(
        home.join("session_index.jsonl"),
        format!("{{\"id\":\"{id}\"}}\n"),
    )
    .unwrap();
    let session_id = codex_archive_session_id(&archive, &rollout).unwrap();

    delete_codex_virtual_session(&root, &session_id).unwrap();

    assert!(!rollout.exists());
    assert!(!fs::read_to_string(home.join("session_index.jsonl"))
        .unwrap()
        .contains(id));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deletes_catalog_only_codex_workspace() {
    let home = test_dir("codex-catalog-workspace-delete");
    let root = home.join("sessions");
    let sqlite_dir = home.join("sqlite");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&sqlite_dir).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let db = sqlite_dir.join("codex-test.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "create table local_thread_catalog (thread_id text, display_title text, cwd text);",
    )
    .unwrap();
    conn.execute(
        "insert into local_thread_catalog values (?1, '残留索引', 'C:\\Catalog')",
        [id],
    )
    .unwrap();
    drop(conn);

    delete_codex_workspace(&root, "C:\\Catalog").unwrap();

    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row("select count(*) from local_thread_catalog", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn restores_codex_session_from_state_index_and_history() {
    let home = test_dir("codex-restore");
    let root = home.join("sessions");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let db = home.join("state_5.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
            "create table threads (id text, rollout_path text, cwd text, title text, created_at integer, updated_at integer, first_user_message text);",
        )
        .unwrap();
    conn.execute(
            "insert into threads values (?1, '', 'C:\\Workspace\\Demo', '数据库标题', 1700000000, 1700000100, '第一条输入')",
            [id],
        )
        .unwrap();
    drop(conn);
    fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"{id}\",\"thread_name\":\"索引标题\",\"updated_at\":\"2023-11-14T22:16:00Z\"}}\n"
            ),
        )
        .unwrap();
    fs::write(
        home.join("history.jsonl"),
        format!("{{\"session_id\":\"{id}\",\"ts\":1700000050,\"text\":\"恢复的用户输入\"}}\n"),
    )
    .unwrap();

    let summaries = collect_codex_source(&test_source(), &root, 8);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].session_id, virtual_session_id("index", id));
    assert_eq!(summaries[0].title, "索引标题");
    assert_eq!(summaries[0].workspace_directory, "C:\\Workspace\\Demo");
    assert_eq!(summaries[0].message_count, 1);

    let loaded = load_codex_index_session(
        &test_source(),
        &root,
        &summaries[0].session_id,
        &summaries[0].workspace_hash,
    )
    .unwrap();
    assert_eq!(loaded.history.len(), 2);
    assert!(loaded.history[1].message.content[0]
        .text
        .contains("恢复的用户输入"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn normalizes_extended_windows_workspace_paths() {
    assert_eq!(
        normalize_codex_workspace(r"\\?\C:\Workspace\Target\"),
        r"c:\workspace\target"
    );
    assert_eq!(
        normalize_codex_workspace(r"\\?\UNC\Server\Share\Project"),
        r"\\server\share\project"
    );
}

#[test]
fn rejects_empty_codex_workspace() {
    let root = test_dir("codex-empty-workspace");
    assert!(delete_codex_workspace(&root, "").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn keeps_unidentified_codex_rollout_for_retry() {
    let home = test_dir("codex-unidentified");
    let root = home.join("sessions");
    fs::create_dir_all(&root).unwrap();
    let rollout = root.join("rollout-without-id.jsonl");
    fs::write(
        &rollout,
        r#"{"type":"session_meta","payload":{"cwd":"C:\\Workspace\\Target"}}
"#,
    )
    .unwrap();

    assert!(delete_codex_workspace(&root, "C:\\Workspace\\Target").is_err());
    assert!(rollout.exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn removes_thread_rows_from_related_tables() {
    let dir = test_dir("codex-db");
    let path = dir.join("state_5.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "create table threads (id text primary key, rollout_path text, cwd text);\
             create table thread_spawn_edges (parent_thread_id text, child_thread_id text);\
             create table logs (thread_id text, body text);\
             insert into threads values ('remove', 'a', 'c'), ('keep', 'b', 'd');\
             insert into thread_spawn_edges values ('remove', 'keep'), ('keep', 'remove');\
             insert into logs values ('remove', 'x'), ('keep', 'y');",
    )
    .unwrap();
    drop(conn);

    delete_thread_rows_from_db(&path, &HashSet::from(["remove".to_string()])).unwrap();
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "select count(*) from threads where id = 'remove'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("select count(*) from thread_spawn_edges", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "select count(*) from logs where thread_id = 'remove'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deletes_workspace_files_and_orphan_codex_threads() {
    let home = test_dir("codex-workspace");
    let root = home.join("sessions");
    let day = root.join("2026").join("07").join("27");
    fs::create_dir_all(&day).unwrap();
    let removed_id = "11111111-1111-4111-8111-111111111111";
    let orphan_id = "22222222-2222-4222-8222-222222222222";
    let keep_id = "33333333-3333-4333-8333-333333333333";
    let rollout = day.join(format!("rollout-{removed_id}.jsonl"));
    fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{removed_id}\",\"cwd\":\"C:\\\\Workspace\\\\Target\"}}}}\n"
            ),
        )
        .unwrap();
    fs::write(
        home.join("session_index.jsonl"),
        format!(
            "{{\"id\":\"{removed_id}\"}}\n{{\"id\":\"{orphan_id}\"}}\n{{\"id\":\"{keep_id}\"}}\n"
        ),
    )
    .unwrap();
    fs::write(
            home.join("history.jsonl"),
            format!(
                "{{\"session_id\":\"{removed_id}\",\"text\":\"remove\"}}\n{{\"session_id\":\"{keep_id}\",\"text\":\"keep\"}}\n"
            ),
        )
        .unwrap();

    let db = home.join("state_5.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("create table threads (id text primary key, rollout_path text, cwd text);")
        .unwrap();
    for (id, cwd) in [
        (removed_id, "C:\\Workspace\\Target"),
        (orphan_id, "C:\\Workspace\\Target"),
        (keep_id, "C:\\Workspace\\Other"),
    ] {
        conn.execute(
            "insert into threads (id, rollout_path, cwd) values (?1, '', ?2)",
            rusqlite::params![id, cwd],
        )
        .unwrap();
    }
    drop(conn);

    delete_codex_workspace(&root, "C:\\Workspace\\Target").unwrap();
    assert!(!rollout.exists());
    let index = fs::read_to_string(home.join("session_index.jsonl")).unwrap();
    assert!(index.contains(keep_id));
    assert!(!index.contains(removed_id));
    let history = fs::read_to_string(home.join("history.jsonl")).unwrap();
    assert!(!history.contains(removed_id));
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "select count(*) from threads where cwd = ?1",
            ["C:\\Workspace\\Target"],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "select count(*) from threads where id = ?1",
            [keep_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn keeps_rollout_file_when_codex_metadata_cleanup_fails() {
    let home = test_dir("codex-retry");
    let root = home.join("sessions");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let rollout = root.join(format!("rollout-{id}.jsonl"));
    fs::write(
            &rollout,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"C:\\\\Workspace\\\\Target\"}}}}\n"
            ),
        )
        .unwrap();
    // 用同名目录模拟索引被占用/不可写，元数据阶段必须失败且保留正文供重试。
    fs::create_dir(home.join("session_index.jsonl")).unwrap();

    assert!(delete_codex_session(&root, &rollout).is_err());
    assert!(rollout.exists());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deletes_index_when_sessions_directory_is_missing() {
    let home = test_dir("codex-index-only");
    let root = home.join("sessions");
    let id = "11111111-1111-4111-8111-111111111111";
    let keep = "22222222-2222-4222-8222-222222222222";
    fs::write(home.join("session_index.jsonl"), format!("{{\"id\":\"{id}\",\"thread_name\":\"remove\"}}\n{{\"id\":\"{keep}\",\"thread_name\":\"keep\"}}\n")).unwrap();
    assert!(codex_metadata_for_id(&root, id).unwrap().is_some());
    delete_codex_virtual_session(&root, &virtual_session_id("index", id)).unwrap();
    assert!(codex_metadata_for_id(&root, id).unwrap().is_none());
    assert!(codex_metadata_for_id(&root, keep).unwrap().is_some());
    assert!(!root.exists());
    fs::remove_dir_all(home).unwrap();
}

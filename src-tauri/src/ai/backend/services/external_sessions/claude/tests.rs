use super::*;

#[test]
fn batch_delete_preserves_other_sessions_and_unrelated_last_session_fields() {
    let home = test_dir("claude-batch");
    let root = home.join("projects");
    fs::create_dir_all(&root).unwrap();
    let first = "11111111-1111-4111-8111-111111111111";
    let second = "22222222-2222-4222-8222-222222222222";
    let kept = "33333333-3333-4333-8333-333333333333";
    let history = [(first, "delete"), (second, "delete"), (kept, "keep")]
        .iter()
        .map(|(id, cwd)| {
            serde_json::json!({"sessionId": id, "project": cwd, "display": "input"}).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(home.join("history.jsonl"), history).unwrap();
    let config = serde_json::json!({"projects": {"keep": {
        "lastSessionId": kept, "lastSessionFirstPrompt": "keep prompt", "lastHintSessionId": first
    }}});
    fs::write(home.join(".claude.json"), config.to_string()).unwrap();
    delete_claude_workspace(&test_source(), &root, "claude-test:delete").unwrap();
    let metadata = claude_metadata_result(&root).unwrap();
    assert_eq!(metadata.len(), 1);
    assert!(metadata.contains_key(kept));
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(config["projects"]["keep"]["lastSessionId"], kept);
    assert_eq!(
        config["projects"]["keep"]["lastSessionFirstPrompt"],
        "keep prompt"
    );
    assert!(config["projects"]["keep"]
        .get("lastHintSessionId")
        .is_none());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn failed_index_cleanup_keeps_body_for_retry() {
    let home = test_dir("claude-failed-delete");
    let root = home.join("projects");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let body = root.join(format!("{id}.jsonl"));
    fs::write(&body, "{}\n").unwrap();
    fs::write(home.join(".claude.json"), "broken config").unwrap();
    assert!(delete_claude_session(&root, Some(&body), id).is_err());
    assert!(body.exists());
    fs::write(home.join(".claude.json"), "{}").unwrap();
    delete_claude_session(&root, Some(&body), id).unwrap();
    assert!(!body.exists());
    fs::remove_dir_all(home).unwrap();
}

fn no_test_root() -> Option<PathBuf> {
    None
}

fn test_source() -> SourceDef {
    SourceDef {
        prefix: "claude-test:",
        source: "claude",
        root: no_test_root,
        layout: Layout::File { depth: 4 },
        parse: parse_claude,
        scan: scan_claude_summary,
    }
}

fn test_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("kirohub-{label}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn streams_claude_page_and_counts_expanded_content_blocks() {
    let dir = test_dir("claude-page");
    let path = dir.join("session.jsonl");
    let lines = [
        serde_json::json!({
            "type": "user",
            "cwd": "C:\\Workspace\\Claude",
            "message": { "content": "第一条" }
        }),
        serde_json::json!({
            "type": "assistant",
            "message": { "content": [
                { "type": "thinking", "thinking": "第二条" },
                { "type": "text", "text": "第三条" }
            ] }
        }),
        serde_json::json!({
            "type": "user",
            "message": { "content": "第四条" }
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

    let page = load_claude_file_page(&test_source(), &path, "session", 2, 2).unwrap();
    assert_eq!(page.total_messages, 5);
    assert_eq!(page.title, "第一条");
    assert_eq!(page.history.len(), 2);
    assert_eq!(page.history[0].message.content[0].text, "第二条");
    assert_eq!(page.history[1].message.content[0].text, "第三条");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restores_claude_history_without_project_jsonl() {
    let home = test_dir("claude-restore");
    let claude_home = home.join(".claude");
    let root = claude_home.join("projects");
    fs::create_dir_all(&root).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    fs::write(
            claude_home.join("history.jsonl"),
            format!(
                "{{\"display\":\"恢复 Claude 输入\",\"pastedContents\":{{\"1\":{{\"content\":\"粘贴正文\"}}}},\"project\":\"C:\\\\Workspace\\\\Claude\",\"sessionId\":\"{id}\",\"timestamp\":1700000000000}}\n"
            ),
        )
        .unwrap();
    fs::write(
            home.join(".claude.json"),
            format!(
                "{{\"projects\":{{\"C:\\\\Workspace\\\\Claude\":{{\"lastSessionId\":\"{id}\",\"lastSessionFirstPrompt\":\"最近输入\",\"lastSessionModified\":1700000100000}}}}}}"
            ),
        )
        .unwrap();

    let summaries = collect_claude_source(&test_source(), &root, 4);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].session_id, virtual_session_id("history", id));
    assert_eq!(summaries[0].title, "恢复 Claude 输入");
    assert_eq!(summaries[0].message_count, 1);

    let loaded = load_claude_history_session(
        &test_source(),
        &root,
        &summaries[0].session_id,
        &summaries[0].workspace_hash,
    )
    .unwrap();
    assert_eq!(loaded.history.len(), 2);
    let text = &loaded.history[1].message.content[0].text;
    assert!(text.contains("恢复 Claude 输入"));
    assert!(text.contains("粘贴正文"));
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deletes_claude_history_recovered_only_from_backups() {
    let home = test_dir("claude-delete-backups");
    let claude_home = home.join(".claude");
    let root = claude_home.join("projects");
    let backups = claude_home.join("backups");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&backups).unwrap();
    let id = "33333333-3333-4333-8333-333333333333";
    let other_id = "44444444-4444-4444-8444-444444444444";
    let original = serde_json::json!({
        "theme": "dark",
        "projects": {
            "C:\\Deleted": {
                "lastSessionId": id,
                "lastSessionFirstPrompt": "deleted prompt",
                "lastSessionModified": 1700000100000i64,
                "allowedTools": ["Read"]
            },
            "C:\\Kept": { "lastSessionId": other_id }
        }
    });
    let paths = [
        backups.join(".claude.json.backup.1"),
        backups.join(".claude.json.backup.2"),
    ];
    for path in &paths {
        fs::write(path, serde_json::to_vec(&original).unwrap()).unwrap();
    }
    let session_id = virtual_session_id("history", id);
    assert!(collect_claude_source(&test_source(), &root, 4)
        .iter()
        .any(|session| session.session_id == session_id));

    delete_claude_session(&root, None, &session_id).unwrap();
    delete_claude_session(&root, None, &session_id).unwrap();
    let summaries = collect_claude_source(&test_source(), &root, 4);
    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].session_id,
        virtual_session_id("history", other_id)
    );
    let mut expected = original;
    let project = expected["projects"]["C:\\Deleted"].as_object_mut().unwrap();
    for key in [
        "lastSessionId",
        "lastSessionFirstPrompt",
        "lastSessionModified",
    ] {
        project.remove(key);
    }
    for path in &paths {
        let actual: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn deletes_claude_body_and_all_known_indexes_together() {
    let home = test_dir("claude-delete");
    let claude_home = home.join(".claude");
    let root = claude_home.join("projects");
    fs::create_dir_all(&root).unwrap();
    let id = "22222222-2222-4222-8222-222222222222";
    let body = root.join(format!("{id}.jsonl"));
    fs::write(
            &body,
            format!(
                "{{\"sessionId\":\"{id}\",\"type\":\"user\",\"cwd\":\"C:\\\\Workspace\",\"message\":{{\"content\":\"body\"}}}}\n"
            ),
        )
        .unwrap();
    fs::write(
            claude_home.join("history.jsonl"),
            format!("{{\"sessionId\":\"{id}\",\"display\":\"history\",\"project\":\"C:\\\\Workspace\",\"timestamp\":1700000000000}}\n"),
        )
        .unwrap();
    fs::write(
            home.join(".claude.json"),
            format!(
                "{{\"projects\":{{\"C:\\\\Workspace\":{{\"lastSessionId\":\"{id}\",\"lastSessionFirstPrompt\":\"prompt\",\"lastSessionModified\":1700000100000}}}}}}"
            ),
        )
        .unwrap();

    delete_claude_session(&root, Some(&body), &format!("{id}.jsonl")).unwrap();
    assert!(!body.exists());
    assert!(!fs::read_to_string(claude_home.join("history.jsonl"))
        .unwrap()
        .contains(id));
    assert!(!fs::read_to_string(home.join(".claude.json"))
        .unwrap()
        .contains(id));
    assert!(claude_metadata_result(&root).unwrap().is_empty());
    fs::remove_dir_all(home).unwrap();
}

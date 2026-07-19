use super::*;
use tempfile::TempDir;

fn user(content: &str) -> Message {
    Message::user(content)
}

fn assistant(content: &str) -> Message {
    Message::assistant(content, vec![])
}

#[tokio::test]
async fn helpers_parse_jsonl_and_detect_prefix_changes() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session::new("cov:jsonl");
    session.messages.push(user("one"));
    session.messages.push(assistant("two"));

    store.save(&session).await.unwrap();
    let path = tmp.path().join("sessions/cov_jsonl.json");
    assert!(is_jsonl_session_file(&path).await.unwrap());

    let raw = tokio::fs::read_to_string(&path).await.unwrap();
    let header = parse_session_header(&raw).unwrap();
    assert_eq!(header.key, "cov:jsonl");
    assert_eq!(header.messages.len(), 2);
    let parsed = parse_session_data(&raw).unwrap();
    assert_eq!(parsed.key, "cov:jsonl");
    assert_eq!(parsed.messages[1].content, "two");

    assert!(
        !persisted_prefix_changed(&path, &session.messages, 2)
            .await
            .unwrap()
    );
    let changed = vec![user("different"), assistant("two")];
    assert!(persisted_prefix_changed(&path, &changed, 2).await.unwrap());
    assert!(
        persisted_prefix_changed(&path, &[user("short")], 2)
            .await
            .unwrap()
    );

    let legacy = tmp.path().join("legacy.json");
    tokio::fs::write(&legacy, br#"{"key":"legacy","messages":[]}"#)
        .await
        .unwrap();
    assert!(!is_jsonl_session_file(&legacy).await.unwrap());
}

#[test]
fn parse_session_header_reports_first_bad_record_and_ignores_trailing_partial() {
    let first_err = match parse_session_header("not-json\n") {
        Ok(_) => panic!("malformed first record should fail"),
        Err(err) => err,
    };
    assert!(first_err.is_syntax() || first_err.is_data(), "{first_err}");

    let trailing_partial = concat!(
        r#"{"type":"snapshot","key":"hdr","messages":[{"role":"user","content":"one"}]}"#,
        "\n",
        r#"{"type":"append","start_index":1,"messages":["#
    );
    let header = parse_session_header(trailing_partial).unwrap();
    assert_eq!(header.key, "hdr");
    assert_eq!(header.messages.len(), 1);
    assert_eq!(header.messages[0].content, "one");
}

#[tokio::test]
async fn append_and_clean_delta_use_append_records_and_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut messages = vec![user("first")];

    store
        .save_clean_delta("cov:delta", &messages, 0, None)
        .await
        .unwrap();
    messages.push(assistant("second"));
    store
        .save_clean_delta("cov:delta", &messages, 1, None)
        .await
        .unwrap();

    let path = tmp.path().join("sessions/cov_delta.json");
    let raw = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(raw.lines().count(), 2, "snapshot plus append: {raw}");
    assert!(raw.lines().nth(1).unwrap().contains(r#""type":"append""#));

    let loaded = store.load("cov:delta").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, "first");
    assert_eq!(loaded.messages[1].content, "second");
}

#[tokio::test]
async fn append_record_rejects_symlinked_session_file() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("session.json");
    let target = tmp.path().join("target.json");
    tokio::fs::write(&target, b"").await.unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &path).unwrap();

    let blocked = user("blocked");
    let record = SessionRecordRef::Append {
        start_index: Some(0),
        messages: vec![message_to_record_ref(&blocked)],
        workflow_run: None,
        workflow_run_cleared: true,
    };
    let err = append_record(&path, &record).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("refusing to append to symlinked session file"),
        "got: {err}"
    );
}

#[tokio::test]
async fn list_load_and_ensure_dir_error_branches_are_exercised() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    assert!(store.list(Some("chat:")).await.unwrap().is_empty());
    let mut chat = Session::new("chat:one");
    chat.messages.push(user(" title "));
    store.save(&chat).await.unwrap();
    let mut other = Session::new("other:one");
    other.messages.push(user("other"));
    store.save(&other).await.unwrap();
    tokio::fs::write(tmp.path().join("sessions/invalid.json"), b"not-json")
        .await
        .unwrap();

    let listed = store.list(Some("chat:")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "chat:one");
    assert_eq!(listed[0].title, "title");
    assert_eq!(listed[0].message_count, 1);
    assert!(store.load("missing").await.unwrap().is_none());

    let file_base = tmp.path().join("file-base");
    tokio::fs::write(&file_base, b"not a dir").await.unwrap();
    let bad_store = FileSessionStore::new(&file_base);
    let err = bad_store.ensure_dir().await.unwrap_err();
    assert!(err.to_string().contains("failed to create sessions dir"));
}

#[tokio::test]
async fn message_record_conversion_preserves_metadata() {
    let mut msg = Message::assistant(
        "answer",
        vec![ToolCall {
            id: "call-1".to_string(),
            name: "grep".to_string(),
            arguments: "{}".to_string(),
        }],
    );
    msg.turn = Some(3);
    msg.is_pinned = true;
    msg.is_manifest = true;
    msg.is_collapsed = true;
    msg.tool_name = Some("grep".to_string());
    msg.input_preview = Some("preview".to_string());
    msg.spill_id = Some("spill".to_string());
    msg.is_error = true;
    msg.stop_reason = Some(StopReason::ToolUse);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "thought".to_string(),
        signature: "sig".to_string(),
    });
    msg.thinking_blocks.push(ThinkingBlock::Redacted {
        data: "secret".to_string(),
    });

    let record = message_to_record(&msg);
    assert_eq!(record.role, "assistant");
    assert_eq!(record.tool_calls[0].name, "grep");
    let round_trip = record_to_message(record);
    assert_eq!(round_trip.role, Role::Assistant);
    assert_eq!(round_trip.turn, Some(3));
    assert!(round_trip.is_pinned);
    assert!(round_trip.is_manifest);
    assert!(round_trip.is_collapsed);
    assert_eq!(round_trip.tool_name.as_deref(), Some("grep"));
    assert_eq!(round_trip.input_preview.as_deref(), Some("preview"));
    assert_eq!(round_trip.spill_id.as_deref(), Some("spill"));
    assert!(round_trip.is_error);
    assert_eq!(round_trip.stop_reason, Some(StopReason::ToolUse));
    assert_eq!(round_trip.thinking_blocks.len(), 2);
}

#[tokio::test]
async fn io_error_mapping_closures_report_real_session_failures() {
    let tmp = TempDir::new().unwrap();

    // append_record: reject_symlink's symlink_metadata map_err on a missing file.
    let missing = tmp.path().join("missing.json");
    let msg = user("x");
    let append = SessionRecordRef::Append {
        start_index: Some(0),
        messages: vec![message_to_record_ref(&msg)],
        workflow_run: None,
        workflow_run_cleared: true,
    };
    let err = append_record(&missing, &append).await.unwrap_err();
    assert!(
        err.to_string().contains("failed to inspect session"),
        "{err}"
    );

    // append_record: OpenOptions::open map_err when the target is a directory.
    let dir_as_file = tmp.path().join("dir-session.json");
    tokio::fs::create_dir(&dir_as_file).await.unwrap();
    let err = append_record(&dir_as_file, &append).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("failed to open session for append"),
        "{err}"
    );

    // write_compacted: write tmp map_err when the .tmp path is an existing directory.
    let compact_path = tmp.path().join("compact.json");
    tokio::fs::create_dir(compact_path.with_extension("tmp"))
        .await
        .unwrap();
    let mut session = Session::new("compact");
    session.messages.push(user("body"));
    let err = write_compacted(&compact_path, &session).await.unwrap_err();
    assert!(err.to_string().contains("failed to write session"), "{err}");

    // write_compacted: rename map_err when replacing an existing directory.
    let rename_path = tmp.path().join("rename.json");
    tokio::fs::create_dir(&rename_path).await.unwrap();
    let err = write_compacted(&rename_path, &session).await.unwrap_err();
    assert!(
        err.to_string().contains("failed to rename session"),
        "{err}"
    );
}

#[tokio::test]
async fn parse_and_probe_error_mapping_closures_report_corrupt_or_unreadable_jsonl() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bad.json");

    // is_jsonl_session_file: File::open map_err.
    let err = is_jsonl_session_file(&path).await.unwrap_err();
    assert!(err.to_string().contains("failed to read session"), "{err}");

    // append_or_compact: parse_session_data map_err after a JSONL-looking corrupt file.
    tokio::fs::write(&path, br#"{"type":"snapshot","key":"oops"#)
        .await
        .unwrap();
    let mut session = Session::new("bad");
    session.messages.push(user("replacement"));
    let err = append_or_compact(&path, &session).await.unwrap_err();
    assert!(err.to_string().contains("failed to parse session"), "{err}");

    // persisted_prefix_changed: read_to_string map_err on a directory.
    let dir = tmp.path().join("as-dir.json");
    tokio::fs::create_dir(&dir).await.unwrap();
    let err = persisted_prefix_changed(&dir, &session.messages, 1)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("failed to read session"), "{err}");
}

#[tokio::test]
async fn w5_session_store_remaining_error_and_default_paths() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    // load: path exists but is not readable as a session file.
    let key = "dir-load";
    let load_path = tmp.path().join("sessions/dir-load.json");
    tokio::fs::create_dir_all(&load_path).await.unwrap();
    let err = store.load(key).await.unwrap_err();
    assert!(err.to_string().contains("failed to read session"), "{err}");

    // parse_session_data: empty JSONL stream falls back to an empty-key session.
    let empty = parse_session_data("").unwrap();
    assert_eq!(empty.key, "");
    assert!(empty.messages.is_empty());

    // is_jsonl_session_file: File::read map_err when the path is a directory.
    let err = is_jsonl_session_file(&load_path).await.unwrap_err();
    assert!(err.to_string().contains("failed to read session"), "{err}");

    // persisted_prefix_changed: parse_session_data map_err on invalid durable JSON.
    let invalid = tmp.path().join("invalid-prefix.json");
    tokio::fs::write(&invalid, b"not json at all")
        .await
        .unwrap();
    let err = persisted_prefix_changed(&invalid, &[user("x")], 1)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("failed to parse session"), "{err}");

    // append_or_compact: read_to_string map_err when an existing jsonl-looking
    // path is later replaced by a directory.
    let as_dir = tmp.path().join("append-dir.json");
    tokio::fs::create_dir(&as_dir).await.unwrap();
    let mut session = Session::new("append-dir");
    session.messages.push(user("body"));
    let err = append_or_compact(&as_dir, &session).await.unwrap_err();
    assert!(err.to_string().contains("failed to read session"), "{err}");
}

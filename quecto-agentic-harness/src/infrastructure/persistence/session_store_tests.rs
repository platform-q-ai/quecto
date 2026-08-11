use super::*;
use tempfile::TempDir;

fn make_message(role: Role, content: &str) -> Message {
    match role {
        Role::System => Message::system(content),
        Role::User => Message::user(content),
        Role::Assistant => Message::assistant(content, vec![]),
        Role::Tool => Message::tool("call", content),
    }
}

#[tokio::test]
async fn empty_conversation_is_not_persisted() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    store.save(&Session::new("test:empty")).await.unwrap();

    assert!(
        !store.exists("test:empty").await.unwrap(),
        "saving an empty conversation must not create a resumable session"
    );
    assert!(
        store.load("test:empty").await.unwrap().is_none(),
        "loading an empty conversation key must behave like an unknown session"
    );
}

#[tokio::test]
async fn empty_delta_is_not_persisted() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    store
        .save_delta("test:empty-delta", &[], 0, None)
        .await
        .unwrap();
    store
        .save_clean_delta("test:empty-clean-delta", &[], 0, None)
        .await
        .unwrap();

    assert!(
        !store.exists("test:empty-delta").await.unwrap(),
        "saving an empty delta must not create a zero-message session"
    );
    assert!(
        !store.exists("test:empty-clean-delta").await.unwrap(),
        "saving an empty clean delta must not create a zero-message session"
    );
}

#[tokio::test]
async fn appending_a_completed_turn_preserves_previously_saved_bytes() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut session = Session::new("test:append");
    session.messages.push(make_message(Role::User, "first"));
    session
        .messages
        .push(make_message(Role::Assistant, "response"));
    store.save(&session).await.unwrap();
    let path = tmp.path().join("sessions/test_append.json");
    let before = tokio::fs::read(&path).await.unwrap();

    session.messages.push(make_message(Role::User, "follow-up"));
    session
        .messages
        .push(make_message(Role::Assistant, "second response"));
    store.save(&session).await.unwrap();

    let after = tokio::fs::read(&path).await.unwrap();
    assert!(
        after.starts_with(&before),
        "normal turns should append new durable records without rewriting previously saved data"
    );
    let appended = std::str::from_utf8(&after[before.len()..]).unwrap();
    let appended_record: serde_json::Value = serde_json::from_str(appended.trim()).unwrap();
    assert_eq!(appended_record["type"], "append");
    assert_eq!(appended_record["messages"].as_array().unwrap().len(), 2);
    assert_eq!(appended_record["messages"][0]["content"], "follow-up");
    assert_eq!(appended_record["messages"][1]["content"], "second response");
    assert!(!appended.contains("first"));
    assert!(!appended.contains("\"content\":\"response\""));

    let loaded = store.load("test:append").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[0].content, "first");
    assert_eq!(loaded.messages[3].content, "second response");
}

#[tokio::test]
async fn interrupted_append_preserves_last_completed_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut session = Session::new("test:interrupted");
    session.messages.push(make_message(Role::User, "first"));
    session
        .messages
        .push(make_message(Role::Assistant, "response"));
    store.save(&session).await.unwrap();

    let path = tmp.path().join("sessions/test_interrupted.json");
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap();
    use tokio::io::AsyncWriteExt;
    file.write_all(br#"{"type":"append","messages":[{"role":"user","content":"lost"}]"#)
        .await
        .unwrap();
    file.flush().await.unwrap();

    let loaded = store.load("test:interrupted").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, "first");
    assert_eq!(loaded.messages[1].content, "response");
}

#[tokio::test]
async fn append_delta_from_stale_cached_index_does_not_mix_replaced_history() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut first_process = Session::new("test:stale");
    first_process
        .messages
        .push(make_message(Role::User, "old 1"));
    first_process
        .messages
        .push(make_message(Role::Assistant, "old 2"));
    store.save(&first_process).await.unwrap();

    let mut second_process = Session::new("test:stale");
    second_process
        .messages
        .push(make_message(Role::User, "replacement"));
    store.save(&second_process).await.unwrap();

    first_process
        .messages
        .push(make_message(Role::User, "stale tail"));
    store
        .save_delta(
            &first_process.key,
            &first_process.messages,
            2,
            first_process.workflow_run.clone(),
        )
        .await
        .unwrap();

    let loaded = store.load("test:stale").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "replacement");
}

/// Pins the shrink SHORTCUT (`previously_persisted > messages.len()` inside
/// `persisted_prefix_changed`): a shorter live history must force a compact
/// rewrite and resume exactly. NOTE (#1073 review): this behavior was also
/// satisfied by the pre-#1072 inline guard, so this test alone does not
/// falsify the masked-prefix detection — that new logic (same-length or
/// longer history with a changed prefix) is pinned by the sibling
/// `masked_pruning_*` tests below; this one guards the shortcut against
/// regressions that would make a shrunk history reach the prefix `zip`
/// (which would index out of bounds or append against a stale prefix).
#[tokio::test]
async fn shorter_history_shortcut_forces_compact_and_resumes_exactly() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session::new("test:shorter-prune");
    for content in ["old 1", "old 2", "old 3"] {
        session.messages.push(make_message(Role::User, content));
    }
    store.save(&session).await.unwrap();

    session.messages = vec![make_message(Role::User, "old 3")];
    store
        .save_delta(&session.key, &session.messages, 3, None)
        .await
        .unwrap();

    let resumed = store.load(&session.key).await.unwrap().unwrap();
    assert_eq!(resumed.messages.len(), 1);
    assert_eq!(resumed.messages[0].role, Role::User);
    assert_eq!(resumed.messages[0].content, "old 3");
}

#[tokio::test]
async fn masked_pruning_compacts_and_resumes_exact_current_history() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session::new("test:masked-prune");
    for content in ["old 1", "old 2", "old 3"] {
        session.messages.push(make_message(Role::User, content));
    }
    store.save(&session).await.unwrap();

    session.messages = vec![
        make_message(Role::User, "old 3"),
        make_message(Role::User, "new prompt"),
        make_message(Role::Assistant, "new answer"),
    ];
    store
        .save_delta(&session.key, &session.messages, 3, None)
        .await
        .unwrap();

    let resumed = store.load(&session.key).await.unwrap().unwrap();
    let contents: Vec<_> = resumed
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(contents, ["old 3", "new prompt", "new answer"]);
}

#[tokio::test]
async fn masked_pruning_with_longer_current_history_resumes_exactly() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session::new("test:masked-prune-longer");
    for content in ["old 1", "old 2", "old 3"] {
        session.messages.push(make_message(Role::User, content));
    }
    store.save(&session).await.unwrap();

    session.messages = vec![
        make_message(Role::User, "old 3"),
        make_message(Role::User, "new prompt"),
        make_message(Role::Assistant, "new answer"),
        make_message(Role::User, "later prompt"),
    ];
    store
        .save_delta(&session.key, &session.messages, 3, None)
        .await
        .unwrap();

    let resumed = store.load(&session.key).await.unwrap().unwrap();
    let contents: Vec<_> = resumed
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        contents,
        ["old 3", "new prompt", "new answer", "later prompt"]
    );
}

#[tokio::test]
async fn replacing_history_compacts_to_the_requested_messages() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut session = Session::new("test:compact");
    for idx in 0..6 {
        session
            .messages
            .push(make_message(Role::User, &format!("message {idx}")));
    }
    store.save(&session).await.unwrap();
    let path = tmp.path().join("sessions/test_compact.json");
    let before = tokio::fs::read(&path).await.unwrap();

    session.messages = session.messages.split_off(4);
    store.save(&session).await.unwrap();

    let after = tokio::fs::read(&path).await.unwrap();
    assert!(
        !after.starts_with(&before),
        "replaced histories should compact instead of appending to obsolete records"
    );
    let compacted_record: serde_json::Value =
        serde_json::from_slice(after.strip_suffix(b"\n").unwrap_or(&after)).unwrap();
    assert_eq!(compacted_record["type"], "snapshot");
    assert_eq!(compacted_record["messages"].as_array().unwrap().len(), 2);

    let loaded = store.load("test:compact").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, "message 4");
    assert_eq!(loaded.messages[1].content, "message 5");
}

#[tokio::test]
async fn load_malformed_json_returns_parse_error() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let dir = tmp.path().join("sessions");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join("bad_json.json"), b"not-json")
        .await
        .unwrap();

    let err = store.load("bad:json").await.unwrap_err();
    assert!(err.to_string().contains("failed to parse session"));
}

#[tokio::test]
async fn list_skips_malformed_and_non_json_files() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut good = Session::new("ok:list");
    good.messages
        .push(make_message(Role::User, "visible title"));
    store.save(&good).await.unwrap();

    let dir = tmp.path().join("sessions");
    tokio::fs::write(dir.join("bad.json"), b"not-json")
        .await
        .unwrap();
    tokio::fs::write(dir.join("ignored.txt"), b"not-json")
        .await
        .unwrap();

    let summaries = store.list(None).await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].key, "ok:list");
    assert_eq!(summaries[0].title, "visible title");
}

#[tokio::test]
async fn load_legacy_snapshot_json_migrates_to_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let dir = tmp.path().join("sessions");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(
        dir.join("legacy_key.json"),
        br#"{"key":"legacy:key","messages":[{"role":"assistant","content":"old","tool_calls":[{"id":"c1","name":"n","arguments":"{}"}]}]}"#,
    )
    .await
    .unwrap();

    let loaded = store.load("legacy:key").await.unwrap().unwrap();
    assert_eq!(loaded.key, "legacy:key");
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].role, Role::Assistant);
    assert_eq!(loaded.messages[0].tool_calls.len(), 1);
}

#[tokio::test]
async fn append_or_compact_rewrites_legacy_snapshot_as_jsonl() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let dir = tmp.path().join("sessions");
    tokio::fs::create_dir_all(&dir).await.unwrap();
    let path = dir.join("legacy_rewrite.json");
    tokio::fs::write(
        &path,
        br#"{"key":"legacy:rewrite","messages":[{"role":"user","content":"old"}]}"#,
    )
    .await
    .unwrap();

    let mut session = Session::new("legacy:rewrite");
    session.messages.push(make_message(Role::User, "new"));
    store.save(&session).await.unwrap();

    let raw = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(raw.starts_with(r#"{"type":"snapshot""#), "raw={raw}");
    let loaded = store.load("legacy:rewrite").await.unwrap().unwrap();
    assert_eq!(loaded.messages[0].content, "new");
}

#[tokio::test]
async fn save_clean_delta_with_zero_watermark_compacts() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let messages = vec![make_message(Role::User, "clean compact")];
    store
        .save_clean_delta("clean:zero", &messages, 0, None)
        .await
        .unwrap();
    let raw = tokio::fs::read_to_string(tmp.path().join("sessions/clean_zero.json"))
        .await
        .unwrap();
    assert!(raw.starts_with(r#"{"type":"snapshot""#), "raw={raw}");
}

// ─── #1460: session-key single-writer ownership on the write path ───────────

/// Writing through the real store to a key whose lock is held by another
/// live process must be refused with an error naming the key and owner —
/// the guard exists to protect this path, not only `acquire` in isolation.
#[tokio::test]
async fn save_to_key_owned_by_another_live_process_is_refused() {
    use std::io::Write;

    let tmp = TempDir::new().unwrap();
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    // Simulated other live process: an independently opened file description
    // holding the exclusive lock, stamped with the parent's (test runner's)
    // pid so the refusal message names a pid that is not this process.
    let owner_pid = std::os::unix::process::parent_id();
    let lock_file = crate::infrastructure::persistence::session_ownership::open_stamp_file(
        &sessions_dir,
        "owned:elsewhere",
    )
    .unwrap();
    lock_file.try_lock().unwrap();
    (&lock_file)
        .write_all(owner_pid.to_string().as_bytes())
        .unwrap();

    let store = FileSessionStore::new(tmp.path());
    let err = <FileSessionStore as SessionStore>::save_clean_delta(
        &store,
        "owned:elsewhere",
        &[],
        0,
        None,
    )
    .await
    .expect_err("empty clean-delta delete owned by another process must be refused");
    let err = err.to_string();
    assert!(err.contains("owned:elsewhere"), "must name the key: {err}");
    assert!(
        err.contains(&owner_pid.to_string()),
        "must name the owning pid: {err}"
    );
}

/// A key stamped by a dead process is reclaimed transparently: the write
/// succeeds and the stamp now records this process.
#[tokio::test]
async fn save_to_key_stamped_by_dead_process_reclaims_and_succeeds() {
    let tmp = TempDir::new().unwrap();
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let dead = child.id();
    child.wait().unwrap();
    let stamp = crate::infrastructure::persistence::session_ownership::ownership_stamp_path(
        &sessions_dir,
        "owned:dead",
    );
    std::fs::write(&stamp, dead.to_string()).unwrap();

    let store = FileSessionStore::new(tmp.path());
    let messages = vec![make_message(Role::User, "reclaimed")];
    store
        .save_clean_delta("owned:dead", &messages, 0, None)
        .await
        .expect("a key stamped by a dead process must be reclaimable");
    let contents = std::fs::read_to_string(&stamp).unwrap();
    assert!(contents.contains(&std::process::id().to_string()));
}

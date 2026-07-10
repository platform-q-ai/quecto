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

#[tokio::test]
async fn shorter_pruned_history_compacts_and_resumes_exactly() {
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

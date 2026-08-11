use super::*;

// Session Steps
// ===========================================================================

/// Helper: ensure a session workspace with session store.
fn ensure_session_workspace(world: &mut QuectoWorld) {
    if world.session_workspace.is_none() {
        let td = TempDir::new().expect("failed to create temp dir");
        let ws = td.path().to_path_buf();
        world.session_store = Some(FileSessionStore::new(&ws));
        world.session_workspace = Some(ws);
        world._temp_dir = Some(td);
    }
}

#[given("a session workspace")]
fn given_session_workspace(world: &mut QuectoWorld) {
    ensure_session_workspace(world);
}

#[given(expr = "no session exists for key {string}")]
fn given_no_session_exists(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let exists = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.exists(&key))
        .unwrap();
    assert!(!exists, "session '{}' should not exist yet", key);
}

#[given(expr = "a session {string} with {int} messages in history")]
fn given_session_with_messages(world: &mut QuectoWorld, key: String, count: usize) {
    ensure_session_workspace(world);
    let store = world.session_store.as_ref().expect("session store not set");

    let mut session = Session::new(&key);
    for i in 0..count {
        let content = format!("Message {}", i + 1);
        if i % 2 == 0 {
            session.messages.push(Message::user(content));
        } else {
            session.messages.push(Message::assistant(content, vec![]));
        }
    }

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();
}

#[given(expr = "a session {string} with messages")]
fn given_session_with_some_messages(world: &mut QuectoWorld, key: String) {
    // Delegate to the parametric version with 2 messages
    given_session_with_messages(world, key, 2);
}

#[given(expr = "a corrupt session file {string}")]
fn given_corrupt_session_file(world: &mut QuectoWorld, filename: String) {
    ensure_session_workspace(world);
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let sessions_dir = ws.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
    std::fs::write(sessions_dir.join(filename), "{ invalid json\n}")
        .expect("write corrupt session");
}

/// Persist a session on disk whose assistant message carries a per-message
/// detail the full message model cannot parse (an unrecognised thinking-block
/// kind). The on-disk JSON shape is an implementation detail kept here, out of
/// the Gherkin: the scenario only speaks of an "unrecognised detail field".
#[given(expr = "a session {string} whose assistant message carries an unrecognised detail field")]
fn given_session_with_unrecognised_detail(world: &mut QuectoWorld, key: String) {
    ensure_session_workspace(world);
    let path = session_file_path(world, &key);
    std::fs::create_dir_all(path.parent().expect("session file should have parent"))
        .expect("create sessions dir");
    let json = format!(
        r#"{{"key":"{key}","messages":[{{"role":"user","content":"what is the answer"}},{{"role":"assistant","content":"42","thinking_blocks":[{{"type":"totally-unknown-variant","x":1}}]}}]}}"#
    );
    std::fs::write(path, json).expect("write session");
}

#[given(expr = "a session {string} with distinct conversation content")]
fn given_session_with_distinct_conversation_content(world: &mut QuectoWorld, key: String) {
    ensure_session_workspace(world);
    let session = Session {
        key,
        messages: vec![
            Message::user("first durable request"),
            Message::assistant("first durable response", vec![]),
            Message::user("second durable request"),
            Message::assistant("second durable response", vec![]),
        ],
        workflow_run: None,
        subagent_roster: Vec::new(),
    };
    world.expected_session_content = session
        .messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    save_session(world, &session);
}

fn session_file_path(world: &QuectoWorld, key: &str) -> PathBuf {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    ws.join("sessions")
        .join(format!("{}.json", key.replace(':', "_")))
}

fn load_session(world: &QuectoWorld, key: &str) -> Session {
    let store = world.session_store.as_ref().expect("session store not set");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.load(key))
        .expect("session load should succeed")
        .unwrap_or_else(|| panic!("expected session {key:?} to exist"))
}

fn save_session(world: &QuectoWorld, session: &Session) {
    let store = world.session_store.as_ref().expect("session store not set");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(session))
        .expect("session save should succeed");
}

fn session_list_entry(world: &QuectoWorld, key: &str) -> quecto::domain::session::SessionSummary {
    let store = world.session_store.as_ref().expect("session store not set");
    let summaries = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.list(None))
        .expect("session list should succeed");
    summaries
        .into_iter()
        .find(|s| s.key == key)
        .unwrap_or_else(|| panic!("expected session list to include {key:?}"))
}

#[then(expr = "the session list entry {string} should have title {string}")]
fn then_session_list_entry_title(world: &mut QuectoWorld, key: String, expected_title: String) {
    assert_eq!(session_list_entry(world, &key).title, expected_title);
}

#[then(expr = "the session list entry {string} should report {int} messages")]
fn then_session_list_entry_count(world: &mut QuectoWorld, key: String, expected_count: usize) {
    assert_eq!(
        session_list_entry(world, &key).message_count,
        expected_count
    );
}

#[given(expr = "the workspace file {string} contains {string}")]
fn given_workspace_file_contains(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, &content).expect("write file");
}

#[when(expr = "the session store creates a session for key {string}")]
fn when_create_session(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let session = Session::new(&key);
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();
}

#[when(expr = "the session store loads session {string}")]
fn when_load_session(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.load(&key))
        .unwrap();
    world.loaded_session = Some(result);
}

#[when(expr = "the session {string} records a completed turn")]
fn when_session_records_completed_turn(world: &mut QuectoWorld, key: String) {
    let mut session = load_session(world, &key);
    world.session_storage_before_turn = Some(
        std::fs::read(session_file_path(world, &key))
            .expect("previously saved session data should be readable"),
    );
    session.messages.push(Message::user("follow-up request"));
    session
        .messages
        .push(Message::assistant("follow-up response", vec![]));
    world.expected_session_content = session
        .messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    save_session(world, &session);
}

#[when(expr = "the session {string} has an interrupted turn")]
fn when_session_has_interrupted_turn(world: &mut QuectoWorld, key: String) {
    let path = session_file_path(world, &key);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open session file for interrupted write");
    file.write_all(br#"{"message":{"role":"user","content":"in-flight request"}}"#)
        .expect("write partial turn");
    file.flush().expect("flush partial turn");
}

#[when(expr = "the session {string} keeps only the latest {int} messages")]
fn when_session_keeps_only_latest_messages(world: &mut QuectoWorld, key: String, count: usize) {
    let mut session = load_session(world, &key);
    world.session_storage_before_replace = Some(
        std::fs::read(session_file_path(world, &key))
            .expect("previously saved session data should be readable"),
    );
    let start = session.messages.len().saturating_sub(count);
    session.messages = session.messages.split_off(start);
    save_session(world, &session);
}

#[when("the session is saved to disk")]
fn when_session_saved_to_disk(world: &mut QuectoWorld) {
    // The Given step already persisted the session via store.save().
    // Verify the session directory contains at least one entry, confirming
    // that the save operation produced durable state on disk.
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let sessions_dir = ws.join("sessions");
    let has_files = std::fs::read_dir(&sessions_dir)
        .expect("sessions directory should exist after save")
        .next()
        .is_some();
    assert!(
        has_files,
        "expected at least one session file in {:?}",
        sessions_dir
    );
}

#[when("the session store is recreated from the same directory")]
fn when_session_store_recreated(world: &mut QuectoWorld) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set")
        .clone();
    world.session_store = Some(FileSessionStore::new(&ws));
}

#[when(expr = "user {string} sends a message on channel {string}")]
fn when_user_sends_message_on_channel(world: &mut QuectoWorld, user_id: String, channel: String) {
    let key = Session::build_key(&channel, &user_id);
    // Create or get session for this routing
    let store = world.session_store.as_ref().expect("session store not set");

    let existing = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.load(&key))
        .unwrap();

    let session = existing.unwrap_or_else(|| Session::new(&key));
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.save(&session))
        .unwrap();

    world.session_keys.insert(user_id, key);
}

#[then(expr = "a session should exist for key {string}")]
fn then_session_exists(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let exists = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.exists(&key))
        .unwrap();
    assert!(exists, "session '{}' should exist", key);
}

#[then(expr = "no session should exist for key {string}")]
fn then_session_does_not_exist(world: &mut QuectoWorld, key: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let exists = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.exists(&key))
        .unwrap();
    assert!(!exists, "session '{key}' should not exist");
}

#[then("the session should be found")]
fn then_session_found(world: &mut QuectoWorld) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed");
    assert!(loaded.is_some(), "expected session to be found");
}

#[then("the session should not be found")]
fn then_session_not_found(world: &mut QuectoWorld) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed");
    assert!(loaded.is_none(), "expected session not to be found");
}

#[then(expr = "the session {string} should reload with {int} messages")]
fn then_session_should_reload_with_messages(world: &mut QuectoWorld, key: String, expected: usize) {
    let loaded = load_session(world, &key);
    assert_eq!(
        loaded.messages.len(),
        expected,
        "expected {expected} messages in {key:?}, got {}",
        loaded.messages.len()
    );
}

#[then(expr = "the session {string} storage should preserve the previously saved data")]
fn then_session_storage_preserves_previously_saved_data(world: &mut QuectoWorld, key: String) {
    let before = world
        .session_storage_before_turn
        .as_ref()
        .expect("previous session storage should have been captured");
    let after = std::fs::read(session_file_path(world, &key))
        .expect("updated session storage should be readable");
    assert!(
        after.starts_with(before),
        "expected completed turn to preserve previously saved session data"
    );
}

#[then(expr = "the session {string} storage should replace the previous data")]
fn then_session_storage_replaces_previous_data(world: &mut QuectoWorld, key: String) {
    let before = world
        .session_storage_before_replace
        .as_ref()
        .expect("previous session storage should have been captured");
    let after = std::fs::read(session_file_path(world, &key))
        .expect("updated session storage should be readable");
    assert!(
        !after.starts_with(before),
        "expected replacing history to rewrite compact storage instead of appending obsolete data"
    );
}

#[then(expr = "the session {string} should reload with the same conversation content")]
fn then_session_should_reload_with_same_conversation_content(world: &mut QuectoWorld, key: String) {
    let loaded = load_session(world, &key);
    let actual: Vec<_> = loaded
        .messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();
    assert_eq!(actual, world.expected_session_content);
}

#[then(expr = "the conversation history should contain {int} messages")]
fn then_conversation_history_contains(world: &mut QuectoWorld, expected: usize) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed")
        .as_ref()
        .expect("session should be found");
    assert_eq!(
        loaded.messages.len(),
        expected,
        "expected {} messages in history, got {}",
        expected,
        loaded.messages.len()
    );
}

#[then(expr = "the file {string} should exist in the session workspace")]
fn then_file_exists_in_session_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set");
    let path = ws.join(&filename);
    assert!(
        path.exists(),
        "file '{}' should exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the session list should include session {string}")]
fn then_session_list_should_include(world: &mut QuectoWorld, expected_name: String) {
    let store = world.session_store.as_ref().expect("session store not set");
    let summaries = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.list(None))
        .expect("session list should succeed");
    assert!(
        summaries.iter().any(|summary| summary.key == expected_name),
        "expected session list to include {expected_name:?}, got {summaries:?}"
    );
}

#[then(expr = "user {string} should have session key {string}")]
fn then_user_has_session_key(world: &mut QuectoWorld, user_id: String, expected_key: String) {
    let key = world
        .session_keys
        .get(&user_id)
        .unwrap_or_else(|| panic!("no session key recorded for user '{}'", user_id));
    assert_eq!(
        key, &expected_key,
        "expected user '{}' to have session key '{}', got '{}'",
        user_id, expected_key, key
    );
}

// ===========================================================================

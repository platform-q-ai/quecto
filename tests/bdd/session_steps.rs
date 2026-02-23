use super::*;

// Session Steps
// ===========================================================================

/// Helper: ensure a session workspace with session store.
fn ensure_session_workspace(world: &mut QuectoWorld) {
    if world.session_workspace.is_none() {
        let td = TempDir::new().expect("failed to create temp dir");
        let ws = td.path().to_path_buf();
        world.session_store = Some(FileSessionStore::new(&ws));
        world.memory_store = Some(MemoryStore::new(&ws));
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

#[when(expr = "the agent writes a memory note {string}")]
fn when_agent_writes_memory(world: &mut QuectoWorld, note: String) {
    let store = world.memory_store.as_ref().expect("memory store not set");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.append(&note))
        .unwrap();
}

#[when("the agent loads identity from the workspace")]
fn when_agent_loads_identity(world: &mut QuectoWorld) {
    let ws = world
        .session_workspace
        .as_ref()
        .expect("session workspace not set")
        .clone();
    let identity = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(memory_store::load_identity(&ws))
        .unwrap();
    world.loaded_identity = Some(identity);
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

#[then("the session should be found")]
fn then_session_found(world: &mut QuectoWorld) {
    let loaded = world
        .loaded_session
        .as_ref()
        .expect("no load was performed");
    assert!(loaded.is_some(), "expected session to be found");
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

#[then(expr = "the memory file should contain {string}")]
fn then_memory_file_contains(world: &mut QuectoWorld, expected: String) {
    let store = world.memory_store.as_ref().expect("memory store not set");
    let content = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(store.read())
        .unwrap();
    assert!(
        content.contains(&expected),
        "expected MEMORY.md to contain '{}', got: {}",
        expected,
        content
    );
}

#[then(expr = "the identity should include {string}")]
fn then_identity_includes(world: &mut QuectoWorld, expected: String) {
    let identity = world.loaded_identity.as_ref().expect("identity not loaded");
    assert!(
        identity.contains(&expected),
        "expected identity to include '{}', got: {}",
        expected,
        identity
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

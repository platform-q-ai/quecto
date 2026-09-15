//! #1976 (D7) — BDD assertions for `new_session` over the real single-client
//! UDS loop: the fresh key's format and distinctness, the switch the
//! `get_state` projection reports, and the session files of the departing
//! and the fresh session.
use super::*;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::session::USER_CHAT_PREFIX;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;

fn response_session_key(world: &QuectoWorld, id: &str) -> String {
    let resp = crate::uds_steps::find_agent_response_by_id(world, id)
        .unwrap_or_else(|| panic!("no response with id {id:?}"));
    assert_eq!(resp["success"], true, "response {id}: {resp}");
    resp["data"]["sessionKey"]
        .as_str()
        .unwrap_or_else(|| panic!("response {id} carries no sessionKey: {resp}"))
        .to_string()
}

#[then(expr = "the new_session response with id {string} should carry a fresh chat session key")]
fn then_new_session_fresh_key(world: &mut QuectoWorld, id: String) {
    let key = response_session_key(world, &id);
    assert!(
        key.starts_with(USER_CHAT_PREFIX),
        "fresh key {key:?} is not a user-chat key"
    );
    let mut parts = key.trim_start_matches(USER_CHAT_PREFIX).splitn(2, '-');
    parts
        .next()
        .and_then(|secs| secs.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("fresh key {key:?} has no wall-clock seconds"));
    let uniq = parts
        .next()
        .unwrap_or_else(|| panic!("fresh key {key:?} has no uniqueness token"));
    assert!(
        u64::from_str_radix(uniq, 16).is_ok(),
        "fresh key {key:?} has a non-hex uniqueness token"
    );
}

#[then(expr = "the session keys of the responses with ids {string} and {string} should differ")]
fn then_session_keys_differ(world: &mut QuectoWorld, first: String, second: String) {
    let a = response_session_key(world, &first);
    let b = response_session_key(world, &second);
    assert_ne!(a, b, "responses {first} and {second} carry the same key");
}

#[then(expr = "the session keys of the responses with ids {string} and {string} should match")]
fn then_session_keys_match(world: &mut QuectoWorld, first: String, second: String) {
    let a = response_session_key(world, &first);
    let b = response_session_key(world, &second);
    assert_eq!(a, b, "responses {first} and {second} carry different keys");
}

#[then(
    expr = "the session file for the key of the response with id {string} should hold {int} messages"
)]
fn then_session_file_for_response_key(world: &mut QuectoWorld, id: String, count: usize) {
    let key = response_session_key(world, &id);
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(&base));
    let session = rt
        .block_on(store.load(&SessionIdentity::from_persisted_key(&key)))
        .expect("failed to load session")
        .unwrap_or_else(|| panic!("no session file for {key:?}"));
    let visible: Vec<&Message> = session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .collect();
    assert_eq!(
        visible.len(),
        count,
        "session {key:?} holds {:?}",
        visible.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
}

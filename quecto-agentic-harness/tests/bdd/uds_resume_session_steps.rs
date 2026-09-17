//! #1977 (D8) — BDD steps for `resume_session` over the real single-client
//! UDS loop: a saved session seeded under the loop's base directory, the
//! resume command with its target as the client spells it, and the
//! acknowledgement's key and message count.
use super::*;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;

#[given(expr = "a saved UDS session {string} with {int} messages in the base directory")]
fn given_saved_uds_session(world: &mut QuectoWorld, key: String, count: usize) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let store = FileSessionStore::new(FlatSessionLayout::new(&base));
    let identity = SessionIdentity::from_persisted_key(key.as_str());
    super::session_scope_steps::record_fixture_home(&base, identity.runtime_key());
    let mut session = Session::new(identity.clone());
    for i in 0..count {
        let content = format!("saved {}", i + 1);
        if i % 2 == 0 {
            session.messages.push(Message::user(content));
        } else {
            session.messages.push(Message::assistant(content, vec![]));
        }
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(store.save(&session))
        .expect("the seeded session saves");
    // The seeding store is not the loop: it holds no claim on the key.
    store.release(&identity);
}

#[when(expr = "I send resume_session {string} with id {string}")]
fn when_send_resume_session(world: &mut QuectoWorld, session: String, id: String) {
    let cmd = serde_json::json!({"type": "resume_session", "id": id, "session": session});
    world.uds_commands.push(cmd.to_string());
}

#[then(
    expr = "the resume_session response with id {string} should carry session key {string} and message count {int}"
)]
fn then_resume_ack(world: &mut QuectoWorld, id: String, key: String, count: u64) {
    let resp = crate::uds_steps::find_agent_response_by_id(world, &id)
        .unwrap_or_else(|| panic!("no response with id {id:?}"));
    assert_eq!(resp["success"], true, "response {id}: {resp}");
    assert_eq!(resp["command"], "resume_session", "response {id}: {resp}");
    assert_eq!(resp["data"]["sessionKey"], key, "response {id}: {resp}");
    assert_eq!(resp["data"]["messageCount"], count, "response {id}: {resp}");
}

#[then(expr = "the response with id {string} should carry the error {string}")]
fn then_response_error(world: &mut QuectoWorld, id: String, error: String) {
    let resp = crate::uds_steps::find_agent_response_by_id(world, &id)
        .unwrap_or_else(|| panic!("no response with id {id:?}"));
    assert_eq!(resp["success"], false, "response {id}: {resp}");
    assert_eq!(resp["error"], error, "response {id}: {resp}");
}

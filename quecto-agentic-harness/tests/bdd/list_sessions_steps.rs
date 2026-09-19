//! #1861 / #1970 — BDD steps for the UDS `list_sessions` command: saved
//! sessions are listed newest first through the composed sessions query,
//! in the summary shape the TUI resume selector reads.

use super::*;
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;

fn list_sessions_response(world: &QuectoWorld) -> serde_json::Value {
    uds_steps::find_agent_response(world, "list_sessions").unwrap_or_else(|| {
        panic!(
            "no list_sessions response\nlines: {:#?}",
            world.agent_events
        )
    })
}

fn listed_sessions(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let response = list_sessions_response(world);
    assert_eq!(response["success"], true, "{response:#?}");
    response["data"]["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("list_sessions data.sessions is not an array: {response:#?}"))
        .clone()
}

fn listed_session<'a>(sessions: &'a [serde_json::Value], key: &str) -> &'a serde_json::Value {
    sessions
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("no listed session {key:?} in {sessions:#?}"))
}

#[given(expr = "a saved session {string} titled {string} with {int} messages updated at {int}")]
fn given_saved_session(
    world: &mut QuectoWorld,
    name: String,
    title: String,
    messages: usize,
    updated_unix_secs: u64,
) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let store = FileSessionStore::new(FlatSessionLayout::new(&base));
    let identity = SessionIdentity::named_cli(&name).expect("valid session name");
    super::session_scope_steps::record_fixture_home(&base, identity.runtime_key());
    let mut session = Session::new(identity);
    for index in 0..messages {
        session.messages.push(if index == 0 {
            Message::user(title.clone())
        } else if index % 2 == 1 {
            Message::assistant(format!("reply {index}"), vec![])
        } else {
            Message::user(format!("follow-up {index}"))
        });
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(store.save(&session))
        .expect("failed to save session");
    let file = FlatSessionLayout::new(&base).session_file(&session.key);
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(updated_unix_secs);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .expect("open saved session")
        .set_modified(modified)
        .expect("stamp modification time");
}

#[then(expr = "the list_sessions response should list the session keys {string} in order")]
fn then_list_sessions_keys_in_order(world: &mut QuectoWorld, keys: String) {
    let listed = listed_sessions(world);
    let actual: Vec<&str> = listed.iter().filter_map(|s| s["key"].as_str()).collect();
    let expected: Vec<&str> = keys.split(',').map(str::trim).collect();
    assert_eq!(actual, expected, "listed keys newest first: {listed:#?}");
}

#[then(
    "every listed session should carry the summary fields key, title, messageCount, updatedUnixSecs and updatedAt"
)]
fn then_every_listed_session_has_summary_fields(world: &mut QuectoWorld) {
    let listed = listed_sessions(world);
    assert!(!listed.is_empty(), "no sessions listed");
    for session in &listed {
        let object = session
            .as_object()
            .unwrap_or_else(|| panic!("summary is not an object: {session:#?}"));
        let mut fields: Vec<&str> = object.keys().map(String::as_str).collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "executionPath",
                "homeState",
                // #2011: the version the row was listed at, echoed on resume.
                "homeVersion",
                "key",
                "messageCount",
                "resumeEligible",
                "title",
                "updatedAt",
                "updatedUnixSecs"
            ],
            "summary shape of {session:#?}"
        );
        assert!(session["key"].is_string());
        assert!(session["title"].is_string());
        assert!(session["messageCount"].is_u64());
        assert_eq!(
            session["updatedUnixSecs"], session["updatedAt"],
            "updatedAt mirrors updatedUnixSecs"
        );
    }
}

#[then(
    expr = "the listed session {string} should show title {string} with {int} messages updated at {int}"
)]
fn then_listed_session_summary(
    world: &mut QuectoWorld,
    key: String,
    title: String,
    messages: u64,
    updated_unix_secs: u64,
) {
    let listed = listed_sessions(world);
    let session = listed_session(&listed, &key);
    assert_eq!(session["title"], title, "{session:#?}");
    assert_eq!(session["messageCount"], messages, "{session:#?}");
    assert_eq!(
        session["updatedUnixSecs"], updated_unix_secs,
        "{session:#?}"
    );
    assert_eq!(session["updatedAt"], updated_unix_secs, "{session:#?}");
}

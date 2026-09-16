//! Step definitions for the D2 (#1971) history/recovery scenarios of
//! `uds_paged_history.feature`: cursor refusal, the `get_messages_tail`
//! alias, unknown message/tool-call refs, offsets past the end and the
//! missing-spill stub fallback, all driven over the REAL UDS server of a
//! seeded persisted session (the plumbing of `uds_paged_history_steps`).

use super::uds_paged_history_steps::{
    PAGED_SESSION, attach_get_messages, connect_paged_client, ensure_query_only_provider_config,
    start_paged_agent, wait_for_paged_event, write_command,
};
use super::*;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::message::{Message, ToolCall};
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use std::time::Duration;

const UNKNOWN_MESSAGE_ID: &str = "00000000-0000-0000-0000-000000000000";

pub(super) fn seed_session(world: &mut QuectoWorld, messages: Vec<Message>) {
    ensure_temp_dir(world);
    ensure_query_only_provider_config(world);
    let base = base_path(world);
    let store = FileSessionStore::new(FlatSessionLayout::new(&base));
    super::session_scope_steps::record_fixture_home(
        &base_path(world),
        &Session::build_key("cli", PAGED_SESSION),
    );
    let session = Session {
        key: SessionIdentity::from_persisted_key(Session::build_key("cli", PAGED_SESSION)),
        messages,
        workflow_run: None,
        subagent_roster: Vec::new(),
    };
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { store.save(&session).await.expect("save seeded session") });
    world.no_session = false;
    world.session_name = Some(PAGED_SESSION.into());
    world._mc_persist = true;
    world.mc_mode = true;
    world.mc_connected_clients = vec![1];
}

pub(super) fn attach(world: &mut QuectoWorld) -> serde_json::Value {
    start_paged_agent(world, PAGED_SESSION);
    connect_paged_client(world, 1);
    attach_get_messages(world, 1)
}

pub(super) fn request(world: &mut QuectoWorld, command: &str, id: &str, cmd: serde_json::Value) {
    write_command(world, 1, &cmd);
    let expected_command = command.to_string();
    let expected_id = id.to_string();
    let response = wait_for_paged_event(
        world,
        1,
        Duration::from_secs(5),
        "the correlated response",
        |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response")
                && event.get("command").and_then(|c| c.as_str()) == Some(expected_command.as_str())
                && event.get("id").and_then(|i| i.as_str()) == Some(expected_id.as_str())
        },
    );
    world._paged_response = Some(response);
}

pub(super) fn response(world: &QuectoWorld) -> &serde_json::Value {
    world._paged_response.as_ref().expect("a recorded response")
}

fn find_message(
    page: &serde_json::Value,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> &serde_json::Value {
    page["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .find(|m| predicate(m))
        .expect("a matching message in the attached page")
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given("a persisted UDS session containing a small message with a tool call")]
fn given_tool_call_session(world: &mut QuectoWorld) {
    let call = Message::assistant(
        "running the tool",
        vec![ToolCall {
            id: "call-small".into(),
            name: "bash".into(),
            arguments: "{\"command\":\"echo hi\"}".into(),
        }],
    );
    seed_session(
        world,
        vec![
            Message::user("please run it"),
            call,
            Message::user("thanks"),
        ],
    );
}

#[given("a persisted UDS session containing a collapsed message whose spill entry is missing")]
fn given_collapsed_without_spill(world: &mut QuectoWorld) {
    let mut collapsed = Message::assistant("recall(\"missing:spill\")", vec![]);
    collapsed.is_collapsed = true;
    collapsed.spill_id = Some("missing:spill".into());
    seed_session(
        world,
        vec![Message::user("earlier"), collapsed, Message::user("later")],
    );
}

// ── When ────────────────────────────────────────────────────────────────────

#[when("a client requests history before an unknown cursor")]
fn when_unknown_cursor(world: &mut QuectoWorld) {
    attach(world);
    request(
        world,
        "get_messages",
        "stale-cursor",
        serde_json::json!({"type": "get_messages", "id": "stale-cursor", "before": UNKNOWN_MESSAGE_ID}),
    );
}

#[when(expr = "a client requests the newest {int} messages through the tail alias")]
fn when_tail_alias(world: &mut QuectoWorld, count: usize) {
    attach(world);
    request(
        world,
        "get_messages_tail",
        "tail-alias",
        serde_json::json!({"type": "get_messages_tail", "id": "tail-alias", "count": count}),
    );
}

#[when("a client requests an unknown message by reference")]
fn when_unknown_message(world: &mut QuectoWorld) {
    attach(world);
    request(
        world,
        "get_message",
        "unknown-ref",
        serde_json::json!({"type": "get_message", "id": "unknown-ref", "messageId": UNKNOWN_MESSAGE_ID}),
    );
}

#[when("a client requests an unknown tool call of the tool-call message")]
fn when_unknown_tool_call(world: &mut QuectoWorld) {
    let page = attach(world);
    let id = find_message(&page, |m| {
        m["toolCalls"].as_array().is_some_and(|c| !c.is_empty())
    })["id"]
        .as_str()
        .expect("message id")
        .to_string();
    request(
        world,
        "get_message",
        "unknown-call",
        serde_json::json!({"type": "get_message", "id": "unknown-call", "messageId": id, "toolCallId": "no-such-call"}),
    );
}

#[when("a client requests the tool-call message content from an offset past its end")]
fn when_offset_past_end(world: &mut QuectoWorld) {
    let page = attach(world);
    let id = find_message(&page, |m| {
        m["toolCalls"].as_array().is_some_and(|c| !c.is_empty())
    })["id"]
        .as_str()
        .expect("message id")
        .to_string();
    request(
        world,
        "get_message",
        "past-end",
        serde_json::json!({"type": "get_message", "id": "past-end", "messageId": id, "offset": 999_999}),
    );
}

#[when("a client requests the collapsed message by its stable reference")]
fn when_collapsed_by_ref(world: &mut QuectoWorld) {
    let page = attach(world);
    let id = find_message(&page, |m| m["collapsed"] == true)["id"]
        .as_str()
        .expect("message id")
        .to_string();
    request(
        world,
        "get_message",
        "collapsed-ref",
        serde_json::json!({"type": "get_message", "id": "collapsed-ref", "messageId": id}),
    );
}

// ── Then ────────────────────────────────────────────────────────────────────

#[then(expr = "the request should be refused with error {string}")]
fn then_refused(world: &mut QuectoWorld, error: String) {
    let response = response(world);
    assert_eq!(response["success"], false, "refused: {response}");
    let text = response["error"].as_str().unwrap_or_default();
    assert!(
        text.contains(&error),
        "error {text:?} should mention {error:?}: {response}"
    );
}

#[then(expr = "the tail alias response should carry {int} messages with the history page shape")]
fn then_tail_shape(world: &mut QuectoWorld, count: usize) {
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["messages"].as_array().map(Vec::len), Some(count));
    assert!(data.get("hasMoreBefore").is_some(), "page shape: {data}");
    assert!(data.get("before").is_some(), "page shape: {data}");
    assert_eq!(data["hasMoreBefore"], true);
    assert!(data["before"].as_str().is_some());
}

#[then("the ranged response should be an empty range at the end of the content")]
fn then_empty_range_at_end(world: &mut QuectoWorld) {
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    let len = data["contentLength"].as_u64().expect("contentLength");
    assert_eq!(data["offset"], len, "{data}");
    assert_eq!(data["nextOffset"], len, "{data}");
    assert_eq!(data["content"], "", "{data}");
    assert_eq!(data["hasMoreContent"], false, "{data}");
}

#[then("the collapsed message should be served as its stub")]
fn then_served_as_stub(world: &mut QuectoWorld) {
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["collapsed"], true, "{data}");
    assert!(
        data["content"]
            .as_str()
            .is_some_and(|c| c.contains("recall(")),
        "{data}"
    );
}

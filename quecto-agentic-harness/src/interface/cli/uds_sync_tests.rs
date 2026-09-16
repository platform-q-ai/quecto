use super::protocol::AgentEvent;
use super::uds_sync::{SYNC_OVERSIZED_ERROR, bounded_response_line, intercept, sync_data};
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::domain::message::{Message, ToolCall};
use crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles;
use crate::interface::cli::uds_session::HISTORY_PAGE_JSON_BUDGET;
use crate::interface::cli::uds_session_handles::SessionReadHandles;

fn publish(handles: &SessionReadHandles, messages: &[Message]) -> LedgerAdvance {
    handles
        .active_session
        .try_write()
        .expect("uncontended")
        .publish(messages)
}

fn position(handles: &SessionReadHandles) -> (u64, u64) {
    let state = handles.active_session.try_read().expect("uncontended");
    (state.conversation().epoch(), state.conversation().rev())
}

async fn sync(handles: &SessionReadHandles, epoch: u64, since_rev: u64) -> serde_json::Value {
    sync_data(&handles.synchronize_transcript, epoch, since_rev).await
}

fn ids(data: &serde_json::Value) -> Vec<String> {
    data["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_delta_frame_carries_the_committed_messages_after_since_rev() {
    let handles = ephemeral_read_handles(&[]);
    let first = Message::user("one");
    let second = Message::assistant("two", vec![]);
    let third = Message::assistant("three", vec![]);
    publish(&handles, &[first.clone(), second.clone(), third.clone()]);
    let data = sync(&handles, 0, 1).await;
    assert_eq!(data["epoch"], 0);
    assert_eq!(data["rev"], 3);
    assert_eq!(data["resync"], false);
    assert_eq!(data["caughtUp"], true);
    assert!(data["nextRev"].is_null());
    assert_eq!(
        ids(&data),
        vec![second.id().to_string(), third.id().to_string()]
    );
}

#[tokio::test]
async fn a_delta_is_cut_at_the_history_frame_budget_with_a_next_rev() {
    let handles = ephemeral_read_handles(&[]);
    let body = "x".repeat(HISTORY_PAGE_JSON_BUDGET / 2 + 1024);
    let messages: Vec<_> = (0..4)
        .map(|_| Message::assistant(body.clone(), vec![]))
        .collect();
    publish(&handles, &messages);
    // Revisions 1..=4; a client at 1 is owed 2, 3 and 4, of which one fits.
    let data = sync(&handles, 0, 1).await;
    assert_eq!(data["resync"], false);
    assert_eq!(ids(&data), vec![messages[1].id().to_string()]);
    assert_eq!(
        data["nextRev"], 3,
        "the revision of the first message left out"
    );
    assert_eq!(data["caughtUp"], false);
    let continued = sync(&handles, 0, 3).await;
    assert_eq!(ids(&continued), vec![messages[3].id().to_string()]);
    assert_eq!(continued["caughtUp"], true);
}

#[tokio::test]
async fn sync_payload_preserves_tool_fields() {
    let handles = ephemeral_read_handles(&[]);
    let mut first_tool = Message::tool("call-1", "error-result");
    first_tool.tool_name = Some("bash".into());
    first_tool.is_error = true;
    let mut second_tool = Message::tool("call-2", "ok-result");
    second_tool.tool_name = Some("read".into());
    second_tool.is_error = false;
    let assistant = Message::assistant(
        "",
        vec![
            ToolCall {
                id: "call-1".into(),
                name: "bash".into(),
                arguments: "echo hi".into(),
            },
            ToolCall {
                id: "call-2".into(),
                name: "read".into(),
                arguments: "Cargo.toml".into(),
            },
        ],
    );
    publish(&handles, &[assistant, first_tool, second_tool]);
    let data = sync(&handles, 0, 0).await;
    assert_eq!(data["messages"][0]["toolCalls"][0]["id"], "call-1");
    assert_eq!(data["messages"][0]["toolCalls"][0]["name"], "bash");
    assert_eq!(data["messages"][0]["toolCalls"][0]["arguments"], "echo hi");
    assert_eq!(data["messages"][0]["toolCalls"][1]["id"], "call-2");
    assert_eq!(data["messages"][0]["toolCalls"][1]["name"], "read");
    assert_eq!(data["messages"][1]["toolCallId"], "call-1");
    assert_eq!(data["messages"][1]["toolName"], "bash");
    assert_eq!(data["messages"][1]["isError"], true);
    assert_eq!(data["messages"][2]["toolCallId"], "call-2");
    assert_eq!(data["messages"][2]["toolName"], "read");
    assert_eq!(data["messages"][2]["isError"], false);
}

#[tokio::test]
async fn a_stale_epoch_is_presented_as_a_resync_at_the_current_position() {
    let handles = ephemeral_read_handles(&[]);
    publish(&handles, &[Message::user("old")]);
    let (epoch, rev) = position(&handles);
    let cleared = handles
        .active_session
        .try_write()
        .expect("uncontended")
        .clear();
    assert_eq!((cleared.epoch, cleared.rev), (epoch + 1, rev));
    let stale = sync(&handles, epoch, rev).await;
    assert_eq!(stale["resync"], true);
    assert_eq!(stale["caughtUp"], true);
    assert!(stale["nextRev"].is_null());
    assert_eq!(stale["epoch"], cleared.epoch);
    assert_eq!(stale["rev"], rev);
    assert_eq!(stale["hasMoreBefore"], false);
    assert!(stale["messages"].as_array().unwrap().is_empty());
    let current = sync(&handles, cleared.epoch, cleared.rev).await;
    assert_eq!(current["resync"], false);
}

#[tokio::test]
async fn a_resync_carries_the_history_page_cursors() {
    let handles = ephemeral_read_handles(&[]);
    let messages: Vec<_> = (0..(super::protocol::HISTORY_PAGE_SIZE + 1))
        .map(|i| Message::user(format!("message-{i}")))
        .collect();
    publish(&handles, &messages);
    let (epoch, rev) = position(&handles);
    let stale = sync(&handles, epoch.wrapping_add(1), rev).await;
    assert_eq!(stale["resync"], true);
    assert_eq!(stale["caughtUp"], true);
    assert_eq!(stale["hasMoreBefore"], true);
    assert_eq!(
        stale["before"].as_str(),
        Some(messages[1].id().to_string().as_str())
    );
    assert_eq!(
        stale["messages"].as_array().unwrap().len(),
        super::protocol::HISTORY_PAGE_SIZE
    );
}

fn client_writer() -> (
    super::uds_ext_protocol::ClientToolRegistry,
    tokio::sync::mpsc::Receiver<String>,
) {
    let clients = super::uds_ext_protocol::new_client_tool_registry();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
    super::uds_ext_protocol::register_client_writer(&clients, 1, tx);
    (clients, rx)
}

#[tokio::test]
async fn the_reader_fast_path_answers_a_parent_local_sync_on_the_client_writer() {
    let handles = ephemeral_read_handles(&[]);
    let committed = Message::user("committed");
    publish(&handles, std::slice::from_ref(&committed));
    let (clients, mut rx) = client_writer();
    let handled = intercept(
        r#"{"type":"sync","id":"fast-1","epoch":0,"sinceRev":0}"#,
        &handles,
        &clients,
        1,
    )
    .await;
    assert!(handled);
    let line = rx.try_recv().expect("a response on the client's writer");
    assert!(line.ends_with('\n'));
    let response: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(response["type"], "response");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "fast-1");
    assert_eq!(response["success"], true);
    assert_eq!(ids(&response["data"]), vec![committed.id().to_string()]);
    assert_eq!(response["data"]["caughtUp"], true);
}

#[tokio::test]
async fn the_reader_fast_path_leaves_other_lines_to_their_own_route() {
    let handles = ephemeral_read_handles(&[Message::user("committed")]);
    let (clients, mut rx) = client_writer();
    for line in [
        r#"{"type":"get_state"}"#,
        r#"{"type":"sync","agent_id":"child","epoch":0,"sinceRev":0}"#,
        r#"{"type":"sync","epoch":"0","sinceRev":0}"#,
        r#"{"type":"sync","epoch":0}"#,
        "garbage",
    ] {
        assert!(!intercept(line, &handles, &clients, 1).await, "{line}");
    }
    assert!(rx.try_recv().is_err(), "nothing was written");
}

#[test]
fn the_oversized_guard_replaces_the_frame_with_the_structured_refusal() {
    assert_eq!(
        SYNC_OVERSIZED_ERROR,
        "sync response exceeds the protocol frame limit; retry with nextRev to continue"
    );
    let event = AgentEvent::ok(
        Some("big-1"),
        "sync",
        Some(serde_json::json!({"messages": ["x".repeat(256)]})),
    );
    let fits = bounded_response_line(&event, Some("big-1"), 1 << 20);
    assert!(fits.ends_with('\n'));
    let fits: serde_json::Value = serde_json::from_str(fits.trim_end()).unwrap();
    assert_eq!(fits["success"], true);
    assert_eq!(
        fits["data"]["messages"][0].as_str().map(str::len),
        Some(256)
    );

    let refused = bounded_response_line(&event, Some("big-1"), 64);
    assert!(refused.ends_with('\n'));
    let refused: serde_json::Value = serde_json::from_str(refused.trim_end()).unwrap();
    assert_eq!(refused["type"], "response");
    assert_eq!(refused["command"], "sync");
    assert_eq!(refused["id"], "big-1");
    assert_eq!(refused["success"], false);
    assert_eq!(refused["error"], SYNC_OVERSIZED_ERROR);
    assert!(
        refused["data"].is_null(),
        "the refusal carries no data: {refused}"
    );
}

#[tokio::test]
async fn the_reader_fast_path_still_reconciles_without_a_registered_writer() {
    let handles = ephemeral_read_handles(&[Message::user("committed")]);
    let clients = super::uds_ext_protocol::new_client_tool_registry();
    assert!(
        intercept(
            r#"{"type":"sync","epoch":0,"sinceRev":0}"#,
            &handles,
            &clients,
            42
        )
        .await
    );
}

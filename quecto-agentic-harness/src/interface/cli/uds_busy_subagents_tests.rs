//! Unit tests for `uds_busy_subagents.rs` — busy-path interception of
//! sub-agent liveness commands (`get_subagents`, child-targeted `sync`).
//!
//! These commands must be answered from the connection's reader task while the
//! serial dispatch loop is occupied by a parent turn; queuing them behind the
//! turn freezes the TUI's left-panel roster and child feed until the parent
//! goes idle (the child-progress-freeze bug, fixed 2026-07-29).

use super::uds_busy_subagents::intercept;
use super::uds_ext_protocol::{
    ClientToolRegistry, new_client_tool_registry, register_client_writer,
};

const CLIENT_ID: u64 = 7;

fn registry_with_writer() -> (ClientToolRegistry, tokio::sync::mpsc::Receiver<String>) {
    let clients = new_client_tool_registry();
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
    register_client_writer(&clients, CLIENT_ID, tx);
    (clients, rx)
}

async fn recv_response(rx: &mut tokio::sync::mpsc::Receiver<String>) -> serde_json::Value {
    let line = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("response within timeout")
        .expect("writer channel open");
    serde_json::from_str(&line).expect("valid JSON response line")
}

#[tokio::test]
async fn get_subagents_is_served_from_the_reader_task_with_id_correlation() {
    let (clients, mut rx) = registry_with_writer();

    let handled = intercept(
        r#"{"type":"get_subagents","id":"gs-1"}"#,
        &None,
        &clients,
        CLIENT_ID,
    )
    .await;

    assert!(
        handled,
        "get_subagents must be served off the dispatch loop"
    );
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "get_subagents");
    assert_eq!(response["id"], "gs-1");
    assert_eq!(response["success"], true);
    assert!(response["data"]["subagents"].as_array().is_some());
    assert_eq!(
        response["data"]["snapshot"], true,
        "busy-path responses carry the #842-style snapshot marker"
    );
}

#[tokio::test]
async fn get_subagents_without_id_is_still_served() {
    let (clients, mut rx) = registry_with_writer();

    assert!(intercept(r#"{"type":"get_subagents"}"#, &None, &clients, CLIENT_ID).await);
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "get_subagents");
    assert!(response["id"].is_null());
}

#[tokio::test]
async fn child_targeted_sync_is_answered_off_the_dispatch_loop() {
    let (clients, mut rx) = registry_with_writer();

    // No registry: the detached forward still resolves to a correlated error
    // response rather than queuing behind the busy dispatch loop or hanging.
    let handled = intercept(
        r#"{"type":"sync","id":"cs-1","agent_id":"child-1","epoch":2,"sinceRev":3}"#,
        &None,
        &clients,
        CLIENT_ID,
    )
    .await;

    assert!(handled, "child-targeted sync must be intercepted");
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "cs-1");
    assert_eq!(response["success"], false);
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("no sub-agent registry"),
    );
}

#[tokio::test]
async fn child_targeted_sync_reports_unknown_child_as_error() {
    let (clients, mut rx) = registry_with_writer();
    let registry = Some(crate::infrastructure::tools::subagent_registry::new_registry());

    assert!(
        intercept(
            r#"{"type":"sync","id":"cs-2","agent_id":"ghost","epoch":0,"sinceRev":0}"#,
            &registry,
            &clients,
            CLIENT_ID,
        )
        .await
    );
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "cs-2");
    assert_eq!(response["success"], false);
}

#[tokio::test]
async fn parent_scoped_sync_falls_through_to_the_ledger_fast_path() {
    let (clients, mut rx) = registry_with_writer();

    // No agent_id: this is the parent's own sync, owned by uds_busy_sync.
    let handled = intercept(
        r#"{"type":"sync","id":"ps-1","epoch":0,"sinceRev":0}"#,
        &None,
        &clients,
        CLIENT_ID,
    )
    .await;

    assert!(!handled, "parent-scoped sync belongs to uds_busy_sync");
    assert!(rx.try_recv().is_err(), "no response may be written here");
}

#[tokio::test]
async fn malformed_child_sync_falls_through_for_dispatch_loop_error_reporting() {
    let (clients, mut rx) = registry_with_writer();

    // agent_id present but epoch/sinceRev missing: leave it to the dispatch
    // loop so the client gets its usual parse/validation error.
    let handled = intercept(
        r#"{"type":"sync","agent_id":"child-1"}"#,
        &None,
        &clients,
        CLIENT_ID,
    )
    .await;

    assert!(!handled);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn unrelated_commands_and_junk_fall_through() {
    let (clients, mut rx) = registry_with_writer();

    for line in [
        r#"{"type":"get_state","id":"x"}"#,
        r#"{"type":"prompt","message":"hi"}"#,
        "not json at all",
    ] {
        assert!(
            !intercept(line, &None, &clients, CLIENT_ID).await,
            "must fall through: {line}"
        );
    }
    assert!(rx.try_recv().is_err());
}

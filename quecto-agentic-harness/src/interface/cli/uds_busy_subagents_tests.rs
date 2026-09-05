//! Unit tests for `uds_busy_subagents.rs` — busy-path interception of
//! sub-agent liveness commands (`get_subagents`, child-targeted `sync`).
//!
//! These commands must be answered from the connection's reader task while the
//! serial dispatch loop is occupied by a parent turn; queuing them behind the
//! turn freezes the TUI's left-panel roster and child feed until the parent
//! goes idle (the child-progress-freeze bug, fixed 2026-07-29).

use super::uds_busy_subagents::{BusySubagentCtx, intercept};
use super::uds_ext_protocol::{
    ClientToolRegistry, new_client_tool_registry, register_client_writer,
};

const CLIENT_ID: u64 = 7;

/// Run one line through the interceptor as client `CLIENT_ID`. Tests that do
/// not observe the broadcast pass `None` and get a private channel.
async fn run(
    line: &str,
    subagents: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    broadcast_tx: Option<&tokio::sync::broadcast::Sender<String>>,
    clients: &ClientToolRegistry,
) -> bool {
    let fallback = tokio::sync::broadcast::channel::<String>(8);
    intercept(BusySubagentCtx {
        line,
        subagents,
        broadcast_tx: broadcast_tx.unwrap_or(&fallback.0),
        clients,
        client_id: CLIENT_ID,
    })
    .await
}

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

    let handled = run(
        r#"{"type":"get_subagents","id":"gs-1"}"#,
        &None,
        None,
        &clients,
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

    assert!(run(r#"{"type":"get_subagents"}"#, &None, None, &clients).await);
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "get_subagents");
    assert!(response["id"].is_null());
}

#[tokio::test]
async fn child_targeted_sync_is_answered_off_the_dispatch_loop() {
    let (clients, mut rx) = registry_with_writer();

    // No registry: the detached forward still resolves to a correlated error
    // response rather than queuing behind the busy dispatch loop or hanging.
    let handled = run(
        r#"{"type":"sync","id":"cs-1","agent_id":"child-1","epoch":2,"sinceRev":3}"#,
        &None,
        None,
        &clients,
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
        run(
            r#"{"type":"sync","id":"cs-2","agent_id":"ghost","epoch":0,"sinceRev":0}"#,
            &registry,
            None,
            &clients
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
    let handled = run(
        r#"{"type":"sync","id":"ps-1","epoch":0,"sinceRev":0}"#,
        &None,
        None,
        &clients,
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
    let handled = run(
        r#"{"type":"sync","agent_id":"child-1"}"#,
        &None,
        None,
        &clients,
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
            !run(line, &None, None, &clients).await,
            "must fall through: {line}"
        );
    }
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn direct_feed_sync_is_served_inline_by_the_child_local_fast_path() {
    // The TUI child feed sends a PLAIN sync (no agent_id) on the child's own
    // socket. Through the full reader dispatch it must be answered inline by
    // uds_busy_sync — never queued behind the dispatch loop (PR #1307 review).
    let (served_inline, response) = crate::interface::cli::busy_reader_dispatch(
        r#"{"type":"sync","id":"feed-9","epoch":1,"sinceRev":0}"#,
    )
    .await;

    assert!(
        served_inline,
        "plain sync must be served on the reader task"
    );
    let response = response.expect("a response line");
    assert_eq!(response["command"], "sync");
    assert_eq!(response["id"], "feed-9");
    assert_eq!(response["success"], true);
}

// ─── delete_all_subagents on the busy path (#1626) ───────────────────────────

fn registry_with_entries(
    names: &[&str],
) -> crate::infrastructure::tools::subagent_registry::SubagentRegistry {
    use crate::infrastructure::tools::subagent_registry::SubagentEntry;
    let registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    for name in names {
        registry.lock().unwrap().insert(
            (*name).to_string(),
            SubagentEntry::new(std::path::PathBuf::from(format!("/tmp/{name}.sock")), 0),
        );
    }
    registry
}

#[tokio::test]
async fn delete_all_subagents_is_served_from_the_reader_task_and_drains_the_registry() {
    let (clients, mut rx) = registry_with_writer();
    let registry = registry_with_entries(&["worker", "reviewer"]);
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);

    let handled = run(
        r#"{"type":"delete_all_subagents","id":"del-1"}"#,
        &Some(registry.clone()),
        Some(&broadcast_tx),
        &clients,
    )
    .await;

    assert!(
        handled,
        "delete_all_subagents must not queue behind a running turn (#1626)"
    );
    assert!(
        registry.lock().unwrap().is_empty(),
        "the registry must be drained synchronously, before the turn ends"
    );
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "delete_all_subagents");
    assert_eq!(response["id"], "del-1");
    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["removed"], 2);

    // Every client receives the authoritative empty survivor set so no
    // later roster refresh can resurrect the deleted agents.
    let broadcast = broadcast_rx.try_recv().expect("state_changed broadcast");
    let event: serde_json::Value = serde_json::from_str(&broadcast).unwrap();
    assert_eq!(event["type"], "subagent_state_changed");
    assert_eq!(event["subagents"].as_array().map(Vec::len), Some(0));

    // A busy-path roster read issued after the delete sees the empty registry.
    assert!(
        run(
            r#"{"type":"get_subagents","id":"gs-after"}"#,
            &Some(registry),
            Some(&broadcast_tx),
            &clients,
        )
        .await
    );
    let after = recv_response(&mut rx).await;
    assert_eq!(after["id"], "gs-after");
    assert_eq!(after["data"]["subagents"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn delete_all_subagents_without_registry_is_a_correlated_error() {
    let (clients, mut rx) = registry_with_writer();

    assert!(
        run(
            r#"{"type":"delete_all_subagents","id":"del-2"}"#,
            &None,
            None,
            &clients
        )
        .await
    );
    let response = recv_response(&mut rx).await;
    assert_eq!(response["command"], "delete_all_subagents");
    assert_eq!(response["id"], "del-2");
    assert_eq!(response["success"], false);
    assert!(
        response["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no sub-agent registry"),
        "unexpected error: {response}"
    );
}

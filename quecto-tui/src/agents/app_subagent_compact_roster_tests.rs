//! Compact `get_subagents` roster refresh must not flicker the left panel.
//!
//! After the tool refactor, `get_subagents` / `get_subagents_all` emit compact
//! rows (`agentId`/`status`/`environmentRef`, optional `unchanged`) instead of
//! the rich `subagent_state_changed` snapshot. The TUI still polls
//! `get_subagents` after every spawn / agent_cmd, so a failed or empty apply
//! wipes live children from the panel even though their sockets stay up.

use super::app_subagent_environment_tests::{env_agent_json, state_changed_line};
use super::tui_harness::*;
use crate::protocol::client::Event;

fn compact_get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": {
            "subagents": agents,
            "sequence": 3,
        },
    })
    .to_string()
}

fn compact_unchanged_get_subagents_response_line() -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": {
            "subagents": [],
            "sequence": 4,
            "unchanged": true,
        },
    })
    .to_string()
}

fn compact_row(id: &str, status: &str, env_ref: Option<&str>) -> serde_json::Value {
    let mut row = serde_json::json!({
        "agentId": id,
        "status": status,
    });
    if let Some(env_ref) = env_ref {
        row["environmentRef"] = serde_json::json!(env_ref);
    }
    row
}

fn rich_local_agent(id: &str, uuid: &str, socket: &str) -> serde_json::Value {
    serde_json::json!({
        "agentId": id,
        "displayName": id,
        "agentUuid": uuid,
        "status": "running",
        "pid": 4242,
        "socketPath": socket,
        "readOnly": false,
    })
}

#[tokio::test]
async fn compact_get_subagents_poll_does_not_wipe_live_panel_rows() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    let before = h.left_panel();
    assert!(
        before.contains("impl"),
        "live state_changed must paint the agent first:\n{before}"
    );
    assert!(
        before.contains("C1"),
        "live state_changed must paint the environment group:\n{before}"
    );

    // This is the post-spawn / post-agent_cmd poll the TUI actually sends.
    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl",
        "running",
        Some("C1"),
    )]));

    let after = h.left_panel();
    assert!(
        after.contains("impl"),
        "compact get_subagents must not drop the live agent from the panel:\n{after}"
    );
    assert!(
        after.contains("C1"),
        "compact get_subagents must keep the environment group:\n{after}"
    );
}

#[tokio::test]
async fn compact_get_subagents_poll_preserves_uuid_identity_and_socket() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let uuid = "uuid-impl";
    let socket = spawn_subagent_socket("impl");
    let socket_path = socket.to_string_lossy().to_string();
    h.event_line(&state_changed_line(vec![rich_local_agent(
        "impl",
        uuid,
        &socket_path,
    )]));

    assert_eq!(
        h.app_mut().subagent_socket_path(uuid).as_deref(),
        Some(socket_path.as_str()),
        "rich live event keys the row on agentUuid and keeps the socket"
    );

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl", "running", None,
    )]));

    assert_eq!(
        h.app_mut().subagent_socket_path(uuid).as_deref(),
        Some(socket_path.as_str()),
        "compact poll keyed by display name must not evict the UUID row or drop the socket"
    );
    assert!(
        h.left_panel().contains("impl"),
        "the painted name must stay after a compact poll:\n{}",
        h.left_panel()
    );
}

#[tokio::test]
async fn unchanged_compact_get_subagents_poll_is_a_no_op() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    h.event_line(&compact_unchanged_get_subagents_response_line());

    let panel = h.left_panel();
    assert!(
        panel.contains("impl"),
        "unchanged:true compact poll must not wipe the panel:\n{panel}"
    );
    assert!(
        panel.contains("C1"),
        "unchanged:true compact poll must keep the environment group:\n{panel}"
    );
}

#[tokio::test]
async fn compact_status_refresh_updates_without_dropping_sticky_fields() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let uuid = "uuid-impl";
    let socket = spawn_subagent_socket("impl-idle");
    let socket_path = socket.to_string_lossy().to_string();
    h.event_line(&state_changed_line(vec![rich_local_agent(
        "impl",
        uuid,
        &socket_path,
    )]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl", "idle", None,
    )]));

    let tracked = &h.app_mut().ac().roster.tracked[uuid];
    assert_eq!(tracked.info.status, "idle");
    assert_eq!(
        tracked.info.socket_path.as_deref(),
        Some(socket_path.as_str())
    );
    assert_eq!(tracked.info.pid, 4242);
    assert!(
        h.left_panel().contains("impl"),
        "idle compact refresh must keep the painted row:\n{}",
        h.left_panel()
    );
}

#[tokio::test]
async fn compact_poll_keeps_rich_environment_identity() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl",
        "running",
        Some("C1"),
    )]));

    let tracked = h
        .app_mut()
        .ac()
        .roster
        .tracked
        .get("uuid-impl")
        .expect("compact poll must rematch onto the live UUID row");
    let env = tracked
        .info
        .environment
        .as_ref()
        .expect("rich environment must survive a compact poll");
    assert_eq!(env.environment_ref, "C1");
    assert_eq!(env.name.as_deref(), Some("pr-env"));
    assert_eq!(env.workspace, "/work/pr-42");
}

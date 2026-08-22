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
    full_compact_get_subagents_response_line(agents)
}

fn full_compact_get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
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

fn delta_compact_get_subagents_response_line(agents: Vec<serde_json::Value>) -> String {
    serde_json::json!({
        "type": "response",
        "command": "get_subagents",
        "success": true,
        "data": {
            "subagents": agents,
            "sequence": 4,
            "unchanged": false,
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
async fn delta_compact_get_subagents_merges_without_deleting_omitted_live_rows() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl-a", "C1"),
        env_agent_json("impl-b", "C2"),
    ]));

    h.event_line(&delta_compact_get_subagents_response_line(vec![
        compact_row("impl-a", "idle", Some("C1")),
    ]));

    let tracked = &h.app_mut().ac().roster.tracked;
    assert_eq!(tracked["uuid-impl-a"].info.status, "idle");
    assert!(tracked.contains_key("uuid-impl-b"));
    let panel = h.left_panel();
    assert!(
        panel.contains("impl-a"),
        "changed row remains visible:\n{panel}"
    );
    assert!(
        panel.contains("impl-b"),
        "delta must not delete omitted live row:\n{panel}"
    );
}

#[tokio::test]
async fn full_compact_get_subagents_snapshot_omitting_row_still_reconciles_removal() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        env_agent_json("impl-a", "C1"),
        env_agent_json("impl-b", "C2"),
    ]));

    h.event_line(&full_compact_get_subagents_response_line(vec![
        compact_row("impl-a", "running", Some("C1")),
    ]));

    let panel = h.left_panel();
    assert!(panel.contains("impl-a"));
    assert!(
        !panel.contains("impl-b"),
        "full snapshot remains authoritative and removes omitted rows:\n{panel}"
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

fn rich_agent_with_sticky_fields(id: &str, uuid: &str, socket: &str) -> serde_json::Value {
    serde_json::json!({
        "agentId": id,
        "displayName": id,
        "agentUuid": uuid,
        "status": "running",
        "lastTool": "bash",
        "lastError": "boom",
        "pid": 4242,
        "socketPath": socket,
        "readOnly": true,
    })
}

#[tokio::test]
async fn compact_status_refresh_preserves_presence_sensitive_sticky_fields() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    let uuid = "uuid-reviewer";
    let socket = spawn_subagent_socket("reviewer-sticky");
    let socket_path = socket.to_string_lossy().to_string();
    h.event_line(&state_changed_line(vec![rich_agent_with_sticky_fields(
        "reviewer",
        uuid,
        &socket_path,
    )]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "reviewer", "idle", None,
    )]));

    let tracked = &h.app_mut().ac().roster.tracked[uuid];
    assert_eq!(tracked.info.status, "idle");
    assert!(
        tracked.info.read_only,
        "absent compact readOnly must preserve observer state"
    );
    assert_eq!(tracked.info.last_tool.as_deref(), Some("bash"));
    assert_eq!(tracked.info.last_error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn legacy_ambiguous_compact_row_does_not_collapse_duplicate_display_names() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![
        rich_local_agent(
            "dup",
            "uuid-dup-a",
            &spawn_subagent_socket("dup-a").to_string_lossy(),
        ),
        rich_local_agent(
            "dup",
            "uuid-dup-b",
            &spawn_subagent_socket("dup-b").to_string_lossy(),
        ),
    ]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "dup", "idle", None,
    )]));

    let tracked = &h.app_mut().ac().roster.tracked;
    assert!(tracked.contains_key("uuid-dup-a"));
    assert!(tracked.contains_key("uuid-dup-b"));
    assert_eq!(
        tracked.len(),
        2,
        "ambiguous legacy compact row must not collapse UUID rows"
    );
    assert_eq!(tracked["uuid-dup-a"].info.status, "idle");
    assert_eq!(tracked["uuid-dup-b"].info.status, "idle");
}

#[tokio::test]
async fn compact_environment_ref_change_does_not_restore_stale_rich_environment() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl",
        "running",
        Some("C2"),
    )]));

    let tracked = &h.app_mut().ac().roster.tracked["uuid-impl"];
    let env = tracked
        .info
        .environment
        .as_ref()
        .expect("new compact ref must remain present");
    assert_eq!(env.environment_ref, "C2");
    assert_ne!(
        env.name.as_deref(),
        Some("pr-env"),
        "rich C1 metadata must not enrich a C2 row"
    );
}

#[tokio::test]
async fn compact_environment_ref_removal_clears_stale_environment_and_backend() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl", "running", None,
    )]));

    let tracked = &h.app_mut().ac().roster.tracked["uuid-impl"];
    assert!(
        tracked.info.environment.is_none(),
        "compact removal without environmentRef must not keep stale C1 metadata"
    );
    assert_eq!(
        tracked.info.execution_backend, None,
        "compact removal without environmentRef must not keep stale script backend"
    );
}

#[tokio::test]
async fn compact_environment_ref_change_clears_stale_backend() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![env_agent_json("impl", "C1")]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "impl",
        "running",
        Some("C2"),
    )]));

    let tracked = &h.app_mut().ac().roster.tracked["uuid-impl"];
    assert_eq!(
        tracked
            .info
            .environment
            .as_ref()
            .map(|e| e.environment_ref.as_str()),
        Some("C2")
    );
    assert_eq!(
        tracked.info.execution_backend, None,
        "compact ref change must not keep stale script backend from C1"
    );
}

#[tokio::test]
async fn compact_dead_inventory_row_does_not_recreate_agent_after_terminal_gc() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![rich_local_agent(
        "killed",
        "uuid-killed",
        &spawn_subagent_socket("killed").to_string_lossy(),
    )]));
    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "killed", "dead", None,
    )]));

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert!(h.app_mut().gc_exited_subagents());
    assert!(!h.left_panel().contains("killed"));

    // Every later agent_cmd operation causes another compact inventory refresh.
    // A dead catalogue row is historical state, not a newly visible agent.
    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "killed", "dead", None,
    )]));

    assert!(
        !h.left_panel().contains("killed"),
        "a later compact inventory refresh must not recreate a killed panel row:\n{}",
        h.left_panel()
    );
    assert!(h.app_mut().ac().roster.tracked.is_empty());
}

#[tokio::test]
async fn legacy_compact_duplicate_name_does_not_revive_retained_terminal_row_before_gc() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event_line(&state_changed_line(vec![rich_local_agent(
        "dup",
        "uuid-dup-old",
        &spawn_subagent_socket("dup-old").to_string_lossy(),
    )]));
    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "dup", "exited", None,
    )]));

    h.event_line(&compact_get_subagents_response_line(vec![compact_row(
        "dup", "idle", None,
    )]));

    let tracked = &h.app_mut().ac().roster.tracked;
    assert_eq!(tracked.len(), 1);
    assert_eq!(tracked["uuid-dup-old"].info.status, "exited");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert!(
        h.app_mut().gc_exited_subagents(),
        "terminal duplicate-name row must stay terminal so normal GC can reclaim it"
    );
    assert!(h.app_mut().ac().roster.tracked.is_empty());
}

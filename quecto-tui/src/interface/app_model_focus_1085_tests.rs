//! Unit tests for routing model changes to the focused sub-agent (#1085).

use super::tui_harness::TuiHarness;
use crate::infrastructure::client::Event;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn get_state_event(model: &str, effort: Option<&str>) -> Event {
    let levels: &[&str] = if model.contains("anthropic") || model.contains("claude") {
        &["low", "medium", "high", "max"]
    } else {
        &["none", "low", "medium", "high", "xhigh"]
    };
    let mut data = serde_json::json!({ "model": model, "effortLevels": levels });
    if let Some(effort) = effort {
        data["effort"] = serde_json::json!(effort);
    }
    Event::Response {
        id: Some("gs".into()),
        command: "get_state".into(),
        success: true,
        data: Some(data),
        error: None,
    }
}

fn command_of_type(commands: &[String], ty: &str) -> Option<serde_json::Value> {
    commands.iter().find_map(|line| {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        (value["type"] == ty).then_some(value)
    })
}

// ── Model routing to focused sub-agent (#1085) ───────────────────────────
// Mirrors the effort routing focus-parity tests above: `/model` must target
// the focused child (not the master), update only that session's footer on
// authoritative state refresh, and re-sync child state.

#[tokio::test]
async fn late_master_set_model_success_does_not_clobber_focused_child_model() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    // Drain connect-on-select traffic so later assertions are about /model only.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), child_rx.recv()).await;
    h.route(
        "child",
        Event::Response {
            id: Some("child-state".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-fable-5",
                "effort": "high",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );

    // Late master set_model success while the child is focused.
    h.event(Event::Response {
        id: None,
        command: "set_model".into(),
        success: true,
        data: Some(serde_json::json!({ "model": "openai-api/gpt-5.4" })),
        error: None,
    });

    assert!(
        h.full_frame().contains("anthropic-api/claude-fable-5"),
        "late master set_model must not replace focused child's model, frame:\n{}",
        h.full_frame()
    );
    assert!(
        !h.notification_messages()
            .iter()
            .any(|m| m.contains("Model switched")),
        "late master set_model must not toast over a focused child"
    );
    // Late master ack may update the master's retained footer without
    // clobbering the focused child's display (#1085).
    assert!(
        h.master_footer_text().contains("openai-api/gpt-5.4"),
        "late master set_model must update master retained footer: {}",
        h.master_footer_text()
    );
    // Active selector marker must still track the focused child.
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-fable-5"),
        "late master set_model must not overwrite focused child's current_model"
    );
}

#[tokio::test]
async fn model_command_with_focused_child_routes_to_child_connection() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    // Clear master + connect noise.
    h.try_drain_commands();
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv()).await
    {}

    h.submit("/model anthropic-api/claude-fable-5");
    let master_cmds = h.try_drain_commands();
    assert!(
        command_of_type(&master_cmds, "set_model").is_none(),
        "focused child model switch must not hit the master stream: {master_cmds:?}"
    );

    let deadline = std::time::Duration::from_secs(2);
    let mut child_cmds = Vec::new();
    while let Ok(Some(line)) = tokio::time::timeout(deadline, child_rx.recv()).await {
        let is_set = command_of_type(std::slice::from_ref(&line), "set_model").is_some();
        child_cmds.push(line);
        if is_set {
            break;
        }
    }
    let cmd = command_of_type(&child_cmds, "set_model")
        .unwrap_or_else(|| panic!("expected child set_model, got {child_cmds:?}"));
    assert_eq!(cmd["model"], "anthropic-api/claude-fable-5");
}

#[tokio::test]
async fn model_command_without_focus_still_targets_master() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/model anthropic-api/claude-fable-5");
    let commands = h.drain_commands().await;
    let cmd = command_of_type(&commands, "set_model")
        .unwrap_or_else(|| panic!("expected master set_model, got {commands:?}"));
    assert_eq!(cmd["model"], "anthropic-api/claude-fable-5");
    assert!(
        h.full_frame().contains("anthropic-api/claude-fable-5"),
        "master footer should optimistically show the new model"
    );
}

#[tokio::test]
async fn model_change_refused_when_focused_child_connection_not_ready() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    // No socket path → connect-on-select cannot install active_cmd_tx.
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("child", "idle", None),
    ]));
    h.select(Some("child"));
    h.submit("/model anthropic-api/claude-fable-5");
    let master_cmds = h.try_drain_commands();
    assert!(
        command_of_type(&master_cmds, "set_model").is_none(),
        "not-ready child must not fall back to master set_model: {master_cmds:?}"
    );
    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m.contains("Selected sub-agent is not ready for model changes yet")),
        "must notify when focused child connection is not ready, got {:?}",
        h.notification_messages()
    );
}

#[tokio::test]
async fn child_set_model_success_updates_only_child_footer_and_resyncs() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv()).await
    {}
    h.route(
        "child",
        Event::Response {
            id: Some("child-state".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-sonnet-4-6",
                "effort": "high",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );

    h.submit("/model anthropic-api/claude-fable-5");
    // Drain the set_model on the child connection.
    let deadline = std::time::Duration::from_secs(2);
    let mut saw_set_model = false;
    while let Ok(Some(line)) = tokio::time::timeout(deadline, child_rx.recv()).await {
        if command_of_type(std::slice::from_ref(&line), "set_model").is_some() {
            saw_set_model = true;
            break;
        }
    }
    assert!(saw_set_model, "expected child set_model");

    // Production set_model acks with data: None (uds.rs). The acknowledgement
    // must toast + resync, but display state remains on the previous model
    // until the authoritative get_state response arrives.
    h.route(
        "child",
        Event::Response {
            id: Some("sm".into()),
            command: "set_model".into(),
            success: true,
            data: None,
            error: None,
        },
    );

    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m.contains("Model switched")),
        "child set_model success (data:None) must toast, got {:?}",
        h.notification_messages()
    );
    assert!(
        h.full_frame().contains("anthropic-api/claude-sonnet-4-6"),
        "bare ack must retain the previous child model until get_state, frame:\n{}",
        h.full_frame()
    );
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-sonnet-4-6"),
        "bare ack must retain the authoritative selector marker"
    );
    assert!(
        h.master_footer_text().contains("openai-api/gpt-5.5"),
        "master retained footer must not be clobbered: {}",
        h.master_footer_text()
    );

    // Successful child set_model must re-sync state on the child connection.
    let mut saw_get_state = false;
    while let Ok(Some(line)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), child_rx.recv()).await
    {
        if command_of_type(std::slice::from_ref(&line), "get_state").is_some() {
            saw_get_state = true;
            break;
        }
    }
    assert!(
        saw_get_state,
        "child set_model success must re-sync with get_state on the child connection"
    );
    let master_cmds = h.try_drain_commands();
    assert!(
        command_of_type(&master_cmds, "get_state").is_none(),
        "child model switch must not re-sync master state: {master_cmds:?}"
    );

    h.route(
        "child",
        Event::Response {
            id: Some("resync".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-fable-5",
                "effort": "low",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );
    assert!(
        h.full_frame().contains("anthropic-api/claude-fable-5"),
        "authoritative get_state must update the child footer, frame:\n{}",
        h.full_frame()
    );
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-fable-5"),
        "authoritative get_state must update the selector marker"
    );
}

#[tokio::test]
async fn failed_child_set_model_keeps_previous_authoritative_model() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv()).await
    {}
    h.route(
        "child",
        Event::Response {
            id: Some("child-state".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-sonnet-4-6",
                "effort": "high",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );

    h.submit("/model anthropic-api/claude-fable-5");
    assert!(
        h.full_frame().contains("anthropic-api/claude-sonnet-4-6"),
        "submission must not optimistically replace the child model"
    );
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-sonnet-4-6")
    );

    h.route(
        "child",
        Event::Response {
            id: Some("sm".into()),
            command: "set_model".into(),
            success: false,
            data: None,
            error: Some("model unavailable".into()),
        },
    );

    assert!(
        h.notification_messages()
            .iter()
            .any(|m| m.contains("Model switch failed: model unavailable")),
        "active child failure must be reported"
    );
    assert!(
        h.full_frame().contains("anthropic-api/claude-sonnet-4-6"),
        "failed switch must retain the previous child footer, frame:\n{}",
        h.full_frame()
    );
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-sonnet-4-6"),
        "failed switch must retain the previous selector marker"
    );
}

#[tokio::test]
async fn late_master_set_model_failure_does_not_toast_over_focused_child() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv()).await
    {}
    h.route(
        "child",
        Event::Response {
            id: Some("child-state".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-fable-5",
                "effort": "high",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );

    h.event(Event::Response {
        id: None,
        command: "set_model".into(),
        success: false,
        data: None,
        error: Some("registry unavailable".into()),
    });

    assert!(
        !h.notification_messages()
            .iter()
            .any(|m| m.contains("Model switch failed")),
        "late master set_model failure must not toast over focused child, got {:?}",
        h.notification_messages()
    );
}

#[tokio::test]
async fn select_agent_restores_current_model_from_session_footer() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    let (socket, mut child_rx) = super::tui_harness::spawn_subagent_socket_with_commands("child");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("child", "idle", None, Some(socket)),
    ]));
    h.select(Some("child"));
    h.try_drain_commands();
    while let Ok(Some(_)) =
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv()).await
    {}
    h.route(
        "child",
        Event::Response {
            id: Some("child-state".into()),
            command: "get_state".into(),
            success: true,
            data: Some(serde_json::json!({
                "model": "anthropic-api/claude-fable-5",
                "effort": "high",
                "effortLevels": ["low", "medium", "high", "max"],
            })),
            error: None,
        },
    );
    assert_eq!(
        h.current_model().as_deref(),
        Some("anthropic-api/claude-fable-5")
    );

    // Return to master: selector marker must restore master's model immediately.
    h.select(None);
    assert_eq!(
        h.current_model().as_deref(),
        Some("openai-api/gpt-5.5"),
        "select_agent(None) must restore master current_model from footer"
    );
}

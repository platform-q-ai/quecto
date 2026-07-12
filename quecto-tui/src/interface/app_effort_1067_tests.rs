//! Unit tests for the runtime reasoning-effort control (#1067):
//! `/effort` command surface, set_effort protocol command, and the
//! footer's effort display.

use super::tui_harness::TuiHarness;
use crate::infrastructure::client::Event;
use crate::interface::components::footer::Footer;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn get_state_event(model: &str, effort: Option<&str>) -> Event {
    // Mirror the agent's real get_state shape: it reports the provider's
    // valid vocabulary in `effortLevels` (the TUI's single source of truth).
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
    commands.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        (v["type"] == ty).then_some(v)
    })
}

// ── builtin command registration ────────────────────────────────────────

#[test]
fn builtin_commands_include_effort() {
    assert!(
        super::builtin_commands().iter().any(|c| c.name == "effort"),
        "builtin_commands must include the /effort command"
    );
}

// ── footer display ──────────────────────────────────────────────────────

#[test]
fn footer_apply_get_state_shows_effort_level() {
    let mut f = Footer::new();
    f.apply_get_state(&serde_json::json!({ "model": "openai-api/gpt-5.5", "effort": "high" }));
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: high"),
        "footer should show 'effort: high', got: {stripped}"
    );
}

#[test]
fn footer_shows_default_effort_when_never_set() {
    let mut f = Footer::new();
    f.apply_get_state(&serde_json::json!({ "model": "openai-api/gpt-5.5" }));
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: default"),
        "footer should show 'effort: default' when effort was never set, got: {stripped}"
    );
}

#[test]
fn footer_shows_default_effort_for_explicit_null() {
    // The agent's real wire shape for a never-set effort is an explicit
    // `"effort": null`; it must render identically to a missing key.
    let mut f = Footer::new();
    f.apply_get_state(
        &serde_json::json!({ "model": "openai-api/gpt-5.5", "effort": serde_json::Value::Null }),
    );
    let lines = crate::interface::component::Component::render(&mut f, 120).join("\n");
    let stripped = super::app_methods::strip_ansi(&lines);
    assert!(
        stripped.contains("effort: default"),
        "explicit-null effort should show 'effort: default', got: {stripped}"
    );
}

#[tokio::test]
async fn footer_updates_when_effort_changes() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    assert!(
        h.full_frame().contains("effort: medium"),
        "footer should show initial effort"
    );
    h.submit("/effort high");
    h.event(Event::Response {
        id: None,
        command: "set_effort".into(),
        success: true,
        data: Some(serde_json::json!({ "effort": "high" })),
        error: None,
    });
    assert!(
        h.full_frame().contains("effort: high"),
        "footer should update live to the new effort, frame:\n{}",
        h.full_frame()
    );
}

// ── /effort <level> direct set ──────────────────────────────────────────

#[tokio::test]
async fn effort_command_with_valid_level_sends_set_effort() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort high");
    let commands = h.drain_commands().await;
    let cmd = command_of_type(&commands, "set_effort")
        .unwrap_or_else(|| panic!("expected a set_effort command, got {commands:?}"));
    assert_eq!(cmd["effort"], "high", "set_effort should carry the level");
}

#[tokio::test]
async fn effort_command_with_invalid_level_is_rejected_listing_valid_levels() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort turbo");
    let commands = h.drain_commands().await;
    assert!(
        command_of_type(&commands, "set_effort").is_none(),
        "invalid level must not send set_effort, got {commands:?}"
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("Invalid effort level"),
        "rejection must be surfaced, frame:\n{frame}"
    );
    assert!(
        frame.contains("valid levels: none, low, medium, high, xhigh"),
        "rejection must list the valid levels for the provider, frame:\n{frame}"
    );
    assert!(
        frame.contains("effort: medium"),
        "previous effort must stay in effect, frame:\n{frame}"
    );
}

// ── /effort selector ─────────────────────────────────────────────────────

#[tokio::test]
async fn effort_selector_lists_openai_vocabulary() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort");
    // Entry-based assertion: frame substrings can't distinguish "high" from
    // "xhigh" and the footer also names a level.
    let entries = h
        .effort_selector_entries()
        .expect("bare /effort should open the effort selector");
    assert_eq!(
        entries,
        ["none", "low", "medium", "high", "xhigh"],
        "OpenAI selector must list exactly the OpenAI vocabulary"
    );
}

#[tokio::test]
async fn effort_selector_lists_anthropic_vocabulary() {
    let mut h = harness().await;
    h.event(get_state_event(
        "anthropic-api/claude-fable-5",
        Some("high"),
    ));
    h.submit("/effort");
    let entries = h
        .effort_selector_entries()
        .expect("bare /effort should open the effort selector");
    assert_eq!(
        entries,
        ["low", "medium", "high", "max"],
        "Anthropic selector must list exactly the Anthropic vocabulary"
    );
}

// ── failure handling ─────────────────────────────────────────────────────

#[tokio::test]
async fn failed_set_effort_response_keeps_previous_footer_value() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.submit("/effort high");
    h.event(Event::Response {
        id: None,
        command: "set_effort".into(),
        success: false,
        data: None,
        error: Some("agent busy".into()),
    });
    let frame = h.full_frame();
    assert!(
        frame.contains("Effort switch failed: agent busy"),
        "failure must be surfaced, frame:\n{frame}"
    );
    assert!(
        frame.contains("effort: medium"),
        "footer must keep the previous effort after a failed switch, frame:\n{frame}"
    );
}

// ── state re-sync (#1067 review) ─────────────────────────────────────────

fn commands_of_type(commands: &[String], ty: &str) -> Vec<serde_json::Value> {
    commands
        .iter()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            (v["type"] == ty).then_some(v)
        })
        .collect()
}

#[tokio::test]
async fn new_session_refetches_state_so_effort_display_cannot_go_stale() {
    // The agent resets its session-scoped effort override on new_session;
    // the TUI must re-fetch state or the footer/selector show a stale level.
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("xhigh")));
    h.drain_commands().await;
    h.submit("/new");
    let commands = h.drain_commands().await;
    assert!(
        !commands_of_type(&commands, "get_state").is_empty(),
        "/new must re-fetch agent state (effort was reset agent-side), got {commands:?}"
    );
}

#[tokio::test]
async fn set_model_success_refetches_state_for_new_vocabulary() {
    // A model switch can change the provider's effort vocabulary; the TUI
    // must re-sync from the agent rather than re-derive it locally.
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("high")));
    h.drain_commands().await;
    h.event(Event::Response {
        id: None,
        command: "set_model".into(),
        success: true,
        data: None,
        error: None,
    });
    let commands = h.drain_commands().await;
    assert!(
        !commands_of_type(&commands, "get_state").is_empty(),
        "set_model success must re-fetch agent state, got {commands:?}"
    );
}

// ── agent-sourced vocabulary (#1067 review) ──────────────────────────────

#[tokio::test]
async fn effort_validation_uses_agent_reported_vocabulary_not_a_local_copy() {
    // The vocabulary comes from get_state's `effortLevels`; a level the
    // agent reports must pass local validation even if it matches no
    // built-in list, and rejection messages must list the agent's levels.
    let mut h = harness().await;
    h.event(Event::Response {
        id: Some("gs".into()),
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "model": "openai-api/gpt-5.5",
            "effort": "low",
            "effortLevels": ["low", "ultra"],
        })),
        error: None,
    });
    h.submit("/effort ultra");
    let commands = h.drain_commands().await;
    assert!(
        command_of_type(&commands, "set_effort").is_some(),
        "an agent-reported level must be accepted, got {commands:?}"
    );
    h.submit("/effort xhigh");
    let commands = h.drain_commands().await;
    assert!(
        command_of_type(&commands, "set_effort").is_none(),
        "a level outside the agent-reported vocabulary must be rejected"
    );
    let frame = h.full_frame();
    assert!(
        frame.contains("valid levels: low, ultra"),
        "rejection must list the agent-reported levels, frame:\n{frame}"
    );
}

#[tokio::test]
async fn late_master_get_state_does_not_replace_focused_child_effort_state() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("child", "idle", None),
    ]));
    h.select(Some("child"));
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

    // A delayed master response must update only master's retained footer.
    h.event(get_state_event("openai-api/gpt-5.5", Some("xhigh")));
    h.submit("/effort");
    assert_eq!(
        h.effort_selector_entries().expect("selector should open"),
        ["low", "medium", "high", "max"],
        "late master state must not replace the focused child's vocabulary"
    );
}

#[tokio::test]
async fn late_master_set_effort_success_does_not_replace_focused_child_effort() {
    let mut h = harness().await;
    h.event(get_state_event("openai-api/gpt-5.5", Some("medium")));
    h.event(super::tui_harness::spawn_start("child"));
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("child", "idle", None),
    ]));
    h.select(Some("child"));
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
        command: "set_effort".into(),
        success: true,
        data: Some(serde_json::json!({ "effort": "xhigh" })),
        error: None,
    });

    assert!(
        h.full_frame().contains("effort: high"),
        "late master success must not replace focused child's effort"
    );
    assert!(
        !h.notification_messages()
            .iter()
            .any(|m| m.contains("Effort set to xhigh")),
        "late master success must not toast the master's level over a focused child"
    );
}

#[tokio::test]
async fn effort_set_before_first_get_state_defers_validation_to_agent() {
    // Before any get_state lands the TUI has no vocabulary; it must not
    // block the command — the agent validates and rejects with the list.
    let mut h = harness().await;
    h.submit("/effort anything");
    let commands = h.drain_commands().await;
    let cmd = command_of_type(&commands, "set_effort")
        .unwrap_or_else(|| panic!("expected set_effort to pass through, got {commands:?}"));
    assert_eq!(cmd["effort"], "anything");
}

// ── Model routing to focused sub-agent (#1085) ───────────────────────────
// Mirrors the effort routing focus-parity tests above: `/model` must target
// the focused child (not the master), update only that session's footer on
// ack, and re-sync child state.

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

    // Production set_model acks with data: None (uds.rs). Footer was already
    // updated optimistically; the success path must still toast + resync.
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
        h.full_frame().contains("anthropic-api/claude-fable-5"),
        "child footer must keep the optimistic model after ack, frame:\n{}",
        h.full_frame()
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

use super::*;
use crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification;
use std::path::PathBuf;

fn wave3_test_entry() -> SubagentEntry {
    SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
}

#[test]
fn wave3_notify_and_broadcast_paths_cover_terminal_and_stall() {
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), wave3_test_entry());
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);

    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"agent_start"}),
    );
    assert!(should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"agent_start"})
    ));
    assert_eq!(entry_workflow_mode(&registry, "bot"), None);
    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"agent_end"}),
    );
    let note = rx.try_recv().unwrap();
    assert!(note.is_completion());
    assert!(matches!(
        registry.lock().unwrap()["bot"].status,
        SubagentStatus::Idle
    ));

    update_entry(&registry, "bot", |e| {
        e.workflow = Some(super::super::subagent_registry::WorkflowSnapshot {
            mode: "active".into(),
            steps_completed: 1,
            steps_total: 2,
        });
        e.stalled_armed = true;
    });
    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"workflow_idle","reason":"exhausted"}),
    );
    assert!(rx.try_recv().unwrap().to_message().contains("stalled"));
}

#[tokio::test]
async fn wave3_agent_error_notification_and_exit_sequence_paths() {
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("bot".into(), wave3_test_entry());
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"response","command":"agent_error","error":"fatal provider error"}),
    );
    assert!(
        rx.try_recv()
            .unwrap()
            .to_message()
            .contains("fatal provider error")
    );
    assert!(!should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"tool_execution_end","isError":true})
    ));
    assert!(!should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"tool_execution_start"})
    ));

    notify_child_exited(&registry, "bot", Some(&tx), None).await;
    assert!(rx.try_recv().unwrap().to_message().contains("exited"));
    assert!(matches!(
        registry.lock().unwrap()["bot"].status,
        SubagentStatus::Exited
    ));
}

#[test]
fn should_broadcast_and_entry_workflow_mode_cover_remaining_arms() {
    let registry = super::super::subagent_registry::new_registry();
    let mut entry = wave3_test_entry();
    entry.workflow = Some(super::super::subagent_registry::WorkflowSnapshot {
        mode: "complete".into(),
        steps_completed: 2,
        steps_total: 2,
    });
    registry.lock().unwrap().insert("bot".into(), entry);
    assert_eq!(
        entry_workflow_mode(&registry, "bot"),
        Some("complete".to_string())
    );
    assert_eq!(entry_workflow_mode(&registry, "missing"), None);

    assert!(should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"agent_end"})
    ));
    assert!(should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"response","command":"agent_error"})
    ));
    assert!(!should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"tool_execution_end","isError":false})
    ));
    assert!(!should_broadcast_state_changed_after_event(
        &serde_json::json!({})
    ));
}

#[tokio::test]
async fn notify_child_exited_missing_agent_sends_sequence_zero_note() {
    let registry = super::super::subagent_registry::new_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    notify_child_exited(&registry, "ghost", Some(&tx), None).await;
    let note = rx.try_recv().unwrap();
    assert_eq!(note.sequence, 0);
    assert!(note.to_message().contains("ghost"));
    assert!(note.to_message().contains("exited unexpectedly"));
}

#[test]
fn notification_fallbacks_cover_missing_agent_paths() {
    let registry = super::super::subagent_registry::new_registry();
    assert_eq!(notification_display_label(&registry, "ghost"), "ghost");
    assert_eq!(
        notification_agent_uuid(&registry, "ghost").as_str(),
        "ghost"
    );
}

#[test]
fn notification_helpers_use_registry_entry_when_present() {
    let registry = super::super::subagent_registry::new_registry();
    let mut entry = wave3_test_entry();
    entry.display_name = "display".into();
    entry.agent_uuid = crate::domain::ids::AgentUuid::new("uuid-123");
    registry.lock().unwrap().insert("uuid-123".into(), entry);
    assert_eq!(notification_display_label(&registry, "uuid-123"), "display");
    assert_eq!(
        notification_agent_uuid(&registry, "uuid-123").as_str(),
        "uuid-123"
    );
}

#[tokio::test]
async fn notify_child_exited_claims_script_cleanup_once() {
    let registry = super::super::subagent_registry::new_registry();
    let mut entry = wave3_test_entry();
    entry.cleanup_environment_id = Some("env-clean".into());
    entry.cleanup_argv = vec!["true".into()];
    registry.lock().unwrap().insert("bot".into(), entry);
    notify_child_exited(&registry, "bot", None, None).await;
    let entry = &registry.lock().unwrap()["bot"];
    assert!(entry.cleanup_environment_id.is_none());
    assert!(entry.cleanup_argv.is_empty());
}

fn poison_registry(registry: &SubagentRegistry) {
    let cloned = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = cloned.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err());
}

#[test]
fn w5_monitor_poisoned_registry_and_notification_edges() {
    let registry = super::super::subagent_registry::new_registry();
    let mut entry = wave3_test_entry();
    entry.workflow = Some(super::super::subagent_registry::WorkflowSnapshot {
        mode: "complete".into(),
        steps_completed: 1,
        steps_total: 1,
    });
    entry.last_error = Some("fatal run failure".into());
    entry.run_error = Some("fatal run failure".into());
    registry.lock().unwrap().insert("bot".into(), entry);
    poison_registry(&registry);
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    // A retained terminal run error suppresses the following agent_end success notification.
    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"agent_end"}),
    );
    assert!(rx.try_recv().is_err());
    assert_eq!(
        entry_workflow_mode(&registry, "bot"),
        Some("complete".into())
    );

    update_entry(&registry, "bot", |e| {
        e.last_error = None;
        e.completion_armed = false;
    });
    apply_and_notify(
        &registry,
        Some(&tx),
        "bot",
        &serde_json::json!({"type":"agent_end"}),
    );
    assert!(
        rx.try_recv().is_err(),
        "unarmed terminal completion is deduped"
    );

    assert_eq!(
        update_entry_next_sequence(&registry, "missing", |_| panic!("must not run")),
        0
    );
}

#[test]
fn w5_notify_from_parsed_error_defaults_and_channel_full_drop() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    notify_from_parsed(
        Some(&tx),
        "bot",
        1,
        &serde_json::json!({}),
        None,
        crate::domain::ids::AgentUuid::new("bot"),
    );
    assert!(rx.try_recv().is_err());

    notify_from_parsed(
        Some(&tx),
        "bot",
        2,
        &serde_json::json!({"type":"response","command":"agent_error"}),
        None,
        crate::domain::ids::AgentUuid::new("bot"),
    );
    assert!(rx.try_recv().unwrap().to_message().contains("agent error"));

    tx.try_send(SequencedSubagentNotification::new(
        3,
        SubagentNotification::Completed {
            agent_id: "prefill".into(),
        },
    ))
    .unwrap();
    notify_from_parsed(
        Some(&tx),
        "bot",
        4,
        &serde_json::json!({"type":"tool_execution_end","isError":true}),
        None,
        crate::domain::ids::AgentUuid::new("bot"),
    );
    assert_eq!(rx.try_recv().unwrap().sequence, 3);
    assert!(rx.try_recv().is_err());
}

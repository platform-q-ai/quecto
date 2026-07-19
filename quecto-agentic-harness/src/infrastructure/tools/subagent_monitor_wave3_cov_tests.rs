use super::*;
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

#[test]
fn wave3_error_notification_and_exit_sequence_paths() {
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
        &serde_json::json!({"type":"tool_execution_end","toolName":"verylongtool","isError":true}),
    );
    assert!(rx.try_recv().unwrap().to_message().contains("verylongtool"));
    assert!(should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"tool_execution_end","isError":true})
    ));
    assert!(!should_broadcast_state_changed_after_event(
        &serde_json::json!({"type":"tool_execution_start"})
    ));

    notify_child_exited(&registry, "bot", Some(&tx));
    assert!(rx.try_recv().unwrap().to_message().contains("exited"));
    assert!(matches!(
        registry.lock().unwrap()["bot"].status,
        SubagentStatus::Exited
    ));
}

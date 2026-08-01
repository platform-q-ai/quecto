// Child tool errors should remain child-local (#1337).

use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentNotification, new_notification_channel, new_registry,
};

fn insert_entry(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    id: &str,
) {
    registry.lock().unwrap().insert(
        id.to_string(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
}

#[tokio::test]
async fn recovered_tool_error_then_agent_end_emits_one_completed() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_start"}),
    );
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"tool_execution_end","toolName":"bash","isError":true}),
    );
    assert!(rx.try_recv().is_err(), "no intermediate parent note");
    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );

    assert!(matches!(
        rx.try_recv().expect("completion notification").notification,
        SubagentNotification::Completed { .. }
    ));
    assert!(rx.try_recv().is_err());
    let entries = registry.lock().unwrap();
    let entry = entries.get("worker").unwrap();
    assert_eq!(entry.status, SubagentStatus::Idle);
    assert!(entry.last_error.is_none());
    assert!(entry.run_error.is_none());
}

#[tokio::test]
async fn multiple_tool_errors_before_completion_emit_only_completion() {
    let registry = new_registry();
    insert_entry(&registry, "worker");
    let (tx, mut rx) = new_notification_channel();

    for tool in ["bash", "edit", "read"] {
        apply_and_notify(
            &registry,
            Some(&tx),
            "worker",
            &serde_json::json!({"type":"tool_execution_end","toolName":tool,"isError":true}),
        );
    }
    assert!(
        rx.try_recv().is_err(),
        "tool errors should produce zero parent notes"
    );

    apply_and_notify(
        &registry,
        Some(&tx),
        "worker",
        &serde_json::json!({"type":"agent_end","messages":[]}),
    );
    assert!(matches!(
        rx.try_recv().expect("completion notification").notification,
        SubagentNotification::Completed { .. }
    ));
    assert!(rx.try_recv().is_err());
}

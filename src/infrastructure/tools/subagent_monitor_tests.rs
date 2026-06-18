use super::*;
use std::path::PathBuf;

fn test_entry() -> SubagentEntry {
    SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
}

// --- apply_event: agent_start ---

#[test]
fn test_agent_start_sets_running() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Idle;
    apply_event(&mut entry, r#"{"type":"agent_start"}"#);
    assert_eq!(entry.status, SubagentStatus::Running);
}

#[test]
fn test_agent_start_clears_last_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Error;
    entry.last_error = Some("old error".to_string());
    apply_event(&mut entry, r#"{"type":"agent_start"}"#);
    assert_eq!(entry.status, SubagentStatus::Running);
    assert!(entry.last_error.is_none());
}

// --- apply_event: agent_end ---

#[test]
fn test_agent_end_sets_idle() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    apply_event(&mut entry, r#"{"type":"agent_end","messages":[]}"#);
    assert_eq!(entry.status, SubagentStatus::Idle);
}

// --- apply_event: tool_execution_start ---

#[test]
fn test_tool_start_sets_running_and_last_tool() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{}}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Running);
    assert_eq!(entry.last_tool.as_deref(), Some("bash"));
}

// --- apply_event: tool_execution_end ---

#[test]
fn test_tool_end_error_sets_error_and_last_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"edit","result":{"content":[]},"isError":true}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Error);
    assert!(entry.last_error.as_ref().unwrap().contains("edit"));
}

#[test]
fn test_tool_end_no_error_keeps_running() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[]},"isError":false}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Running);
}

#[test]
fn test_tool_end_success_clears_last_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    entry.last_error = Some("previous error".to_string());
    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"read","result":{"content":[]},"isError":false}"#,
    );
    assert!(entry.last_error.is_none());
}

// --- apply_event: unknown / malformed ---

#[test]
fn test_unknown_event_ignored() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Idle;
    apply_event(&mut entry, r#"{"type":"token","token":"hello"}"#);
    assert_eq!(entry.status, SubagentStatus::Idle);
}

#[test]
fn test_malformed_json_ignored() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Idle;
    apply_event(&mut entry, "not valid json");
    assert_eq!(entry.status, SubagentStatus::Idle);
}

// --- mark_exited ---

#[test]
fn test_mark_exited() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    mark_exited(&mut entry);
    assert_eq!(entry.status, SubagentStatus::Exited);
}

// --- truncate_string ---

#[test]
fn test_truncate_string_short() {
    assert_eq!(truncate_string("hello", 10), "hello");
}

#[test]
fn test_truncate_string_exact() {
    assert_eq!(truncate_string("hello", 5), "hello");
}

#[test]
fn test_truncate_string_long() {
    let result = truncate_string("hello world", 5);
    assert_eq!(result, "hello…");
}

// --- update_entry ---

#[test]
fn test_update_entry_modifies_registry() {
    let registry = super::super::subagent_registry::new_registry();
    registry.lock().unwrap().insert(
        "bot".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/bot.sock"), 0),
    );
    update_entry(&registry, "bot", |e| {
        e.status = SubagentStatus::Running;
    });
    let entries = registry.lock().unwrap();
    assert_eq!(entries["bot"].status, SubagentStatus::Running);
}

#[test]
fn test_update_entry_missing_agent_is_noop() {
    let registry = super::super::subagent_registry::new_registry();
    // Should not panic
    update_entry(&registry, "nonexistent", |e| {
        e.status = SubagentStatus::Running;
    });
}

// --- pre-filter ---

#[test]
fn test_pre_filter_skips_token_events() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Idle;
    // Token event should be filtered out before JSON parse.
    apply_event(&mut entry, r#"{"type":"token","token":"hello"}"#);
    assert_eq!(entry.status, SubagentStatus::Idle);
}

// --- Sequence of events ---

#[test]
fn test_full_lifecycle() {
    let mut entry = test_entry();
    assert_eq!(entry.status, SubagentStatus::Starting);

    apply_event(&mut entry, r#"{"type":"agent_start"}"#);
    assert_eq!(entry.status, SubagentStatus::Running);

    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{}}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Running);
    assert_eq!(entry.last_tool.as_deref(), Some("bash"));

    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":false}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Running);

    apply_event(&mut entry, r#"{"type":"agent_end","messages":[]}"#);
    assert_eq!(entry.status, SubagentStatus::Idle);

    mark_exited(&mut entry);
    assert_eq!(entry.status, SubagentStatus::Exited);
}

// --- maybe_notify (#523) ---

#[tokio::test]
async fn test_notify_on_agent_end() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"Done"}]}"#;
    maybe_notify(Some(&tx), "worker", line);
    let notif = rx.try_recv().unwrap();
    match notif.notification {
        SubagentNotification::Completed {
            agent_id, summary, ..
        } => {
            assert_eq!(agent_id, "worker");
            assert_eq!(summary, "Done");
        }
        _ => panic!("expected Completed"),
    }
}

#[tokio::test]
async fn test_notify_on_tool_error() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":true}"#;
    maybe_notify(Some(&tx), "worker", line);
    let notif = rx.try_recv().unwrap();
    match notif.notification {
        SubagentNotification::Errored {
            agent_id, error, ..
        } => {
            assert_eq!(agent_id, "worker");
            assert!(error.contains("bash"));
        }
        _ => panic!("expected Errored"),
    }
}

#[tokio::test]
async fn test_no_notify_on_agent_start() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"agent_start"}"#;
    maybe_notify(Some(&tx), "worker", line);
    assert!(rx.try_recv().is_err(), "no notification should be sent");
}

#[tokio::test]
async fn test_no_notify_on_successful_tool_end() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":false}"#;
    maybe_notify(Some(&tx), "worker", line);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_notify_none_tx_is_noop() {
    // Should not panic
    maybe_notify(None, "worker", r#"{"type":"agent_end","messages":[]}"#);
}

#[tokio::test]
async fn test_send_notification_exited() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    send_notification(
        Some(&tx),
        super::super::subagent_registry::SequencedSubagentNotification::new(
            1,
            SubagentNotification::Exited {
                agent_id: "bot".to_string(),
            },
        ),
    );
    let notif = rx.try_recv().unwrap();
    assert_eq!(
        notif,
        super::super::subagent_registry::SequencedSubagentNotification::new(
            1,
            SubagentNotification::Exited {
                agent_id: "bot".to_string(),
            },
        )
    );
}

#[tokio::test]
async fn test_maybe_notify_agent_end() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"agent_end","messages":[{"role":"assistant","content":"done"}]}"#;
    maybe_notify(Some(&tx), "worker", line);
    let notif = rx.try_recv().unwrap();
    match notif.notification {
        SubagentNotification::Completed {
            agent_id, summary, ..
        } => {
            assert_eq!(agent_id, "worker");
            assert!(summary.contains("done"));
        }
        _ => panic!("expected Completed"),
    }
}

#[tokio::test]
async fn test_maybe_notify_tool_error() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"tool_execution_end","toolName":"bash","isError":true}"#;
    maybe_notify(Some(&tx), "worker", line);
    let notif = rx.try_recv().unwrap();
    match notif.notification {
        SubagentNotification::Errored {
            agent_id, error, ..
        } => {
            assert_eq!(agent_id, "worker");
            assert!(error.contains("bash"));
        }
        _ => panic!("expected Errored"),
    }
}

#[tokio::test]
async fn test_maybe_notify_tool_success_no_notification() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"tool_execution_end","toolName":"bash","isError":false}"#;
    maybe_notify(Some(&tx), "worker", line);
    assert!(rx.try_recv().is_err()); // No notification for success
}

#[test]
fn test_maybe_notify_none_tx_is_noop() {
    let line = r#"{"type":"agent_end","messages":[]}"#;
    maybe_notify(None, "worker", line); // should not panic
}

#[test]
fn test_maybe_notify_invalid_json_is_noop() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    maybe_notify(Some(&tx), "worker", "not json");
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_maybe_notify_non_state_event_is_noop() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"token","token":"hello"}"#;
    maybe_notify(Some(&tx), "worker", line);
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_notify_from_parsed_unknown_event_is_noop() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let value = serde_json::json!({"type": "token", "token": "hi"});
    notify_from_parsed(Some(&tx), "worker", 1, &value);
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_notify_from_parsed_no_type_is_noop() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let value = serde_json::json!({"data": "something"});
    notify_from_parsed(Some(&tx), "worker", 1, &value);
    assert!(rx.try_recv().is_err());
}

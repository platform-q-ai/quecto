use super::*;

#[test]
fn test_new_registry_is_empty() {
    let r = new_registry();
    assert!(r.lock().unwrap().is_empty());
}

#[test]
fn verdict_completed_when_idle_and_workflow_complete() {
    let wf = WorkflowSnapshot {
        mode: "complete".into(),
        steps_completed: 7,
        steps_total: 7,
    };
    let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf), None);
    assert_eq!(r.status, VerdictStatus::Completed);
    assert_eq!(
        r.workflow_progress,
        Some(ResultProgress { done: 7, total: 7 })
    );
}

#[test]
fn verdict_incomplete_when_idle_but_workflow_active() {
    let wf = WorkflowSnapshot {
        mode: "active".into(),
        steps_completed: 3,
        steps_total: 7,
    };
    let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf), None);
    assert_eq!(r.status, VerdictStatus::Incomplete);
}

#[test]
fn verdict_incomplete_when_idle_without_workflow() {
    let r = WorkflowResult::derive("idle", Some("idle"), None, None);
    assert_eq!(r.status, VerdictStatus::Incomplete);
    assert!(r.workflow_progress.is_none());
}

#[test]
fn verdict_failed_on_error_and_nonzero_exit() {
    assert_eq!(
        WorkflowResult::derive("error", Some("connection_failed"), None, None).status,
        VerdictStatus::Failed
    );
    assert_eq!(
        WorkflowResult::derive("exited", Some("exit_code_1"), None, None).status,
        VerdictStatus::Failed
    );
    // A clean exit is NOT completion — completion is observed at idle.
    assert_eq!(
        WorkflowResult::derive("exited", Some("exit_code_0"), None, None).status,
        VerdictStatus::Incomplete
    );
}

#[test]
fn verdict_incomplete_on_timeout() {
    assert_eq!(
        WorkflowResult::derive("timeout", None, None, None).status,
        VerdictStatus::Incomplete
    );
}

#[test]
fn error_cause_is_threaded_into_summary_preserving_reason() {
    // #752: the run cause is appended to the verdict summary in one place
    // (derive), and the reason context (`agent_error`) is preserved.
    let r = WorkflowResult::derive("error", Some("agent_error"), None, Some("HTTP 429"));
    assert_eq!(r.status, VerdictStatus::Failed);
    assert_eq!(r.summary, "await error: agent_error — HTTP 429");
}

#[test]
fn redact_secrets_strips_known_patterns_and_bounds_length() {
    // #752 security review: secrets must not cross the trust boundary into
    // the parent context verbatim.
    assert_eq!(
        redact_secrets("auth failed: Authorization: Bearer abc.def.ghi"),
        "auth failed: Authorization: [REDACTED]"
    );
    assert_eq!(
        redact_secrets("bad key sk-ABCDEFGH12345678 rejected"),
        "bad key [REDACTED] rejected"
    );
    assert_eq!(
        redact_secrets("url ?api_key=topsecret&x=1"),
        "url ?[REDACTED]"
    );
    assert_eq!(redact_secrets("token=hunter2"), "[REDACTED]");
    // Non-secret text is preserved verbatim.
    assert_eq!(redact_secrets("usage_limit_reached"), "usage_limit_reached");
    // Over-long causes are truncated.
    let long = "x".repeat(5000);
    let out = redact_secrets(&long);
    assert!(out.ends_with("…[truncated]"));
    assert!(out.len() < 5000);
}

#[test]
fn with_error_redacts_cause_in_both_field_and_summary() {
    let r = AwaitResult::with_error(
        "error",
        Some("agent_error"),
        "bot-1".into(),
        10,
        None,
        Some("Authorization: Bearer sk-secrettoken123"),
    );
    let err = r.error.unwrap();
    assert!(
        !err.contains("sk-secrettoken123"),
        "field leaked secret: {err}"
    );
    assert!(err.contains("[REDACTED]"));
    assert!(
        !r.result.summary.contains("sk-secrettoken123"),
        "summary leaked secret: {}",
        r.result.summary
    );
}

#[test]
fn test_validate_format_valid() {
    assert!(validate_agent_id_format("abc-123_XYZ").is_ok());
}

#[test]
fn test_validate_format_empty() {
    assert!(validate_agent_id_format("").unwrap_err().contains("1-64"));
}

#[test]
fn test_validate_format_too_long() {
    assert!(
        validate_agent_id_format(&"a".repeat(65))
            .unwrap_err()
            .contains("1-64")
    );
}

#[test]
fn test_validate_format_special_chars() {
    assert!(
        validate_agent_id_format("a/b")
            .unwrap_err()
            .contains("[a-zA-Z0-9_-]")
    );
}

// --- SubagentStatus::to_wire_str ---
#[test]
fn test_status_wire_str_values() {
    assert_eq!(SubagentStatus::Starting.to_wire_str(), "starting");
    assert_eq!(SubagentStatus::Idle.to_wire_str(), "idle");
    assert_eq!(SubagentStatus::Running.to_wire_str(), "running");
    assert_eq!(SubagentStatus::Error.to_wire_str(), "error");
    assert_eq!(SubagentStatus::Exited.to_wire_str(), "exited");
}

// --- SubagentStatus ---

#[test]
fn test_status_display_starting() {
    assert_eq!(format!("{}", SubagentStatus::Starting), "Starting");
}

#[test]
fn test_status_display_idle() {
    assert_eq!(format!("{}", SubagentStatus::Idle), "Idle");
}

#[test]
fn test_status_display_running() {
    assert_eq!(format!("{}", SubagentStatus::Running), "Running");
}

#[test]
fn test_status_display_error() {
    assert_eq!(format!("{}", SubagentStatus::Error), "Error");
}

#[test]
fn test_status_display_exited() {
    assert_eq!(format!("{}", SubagentStatus::Exited), "Exited");
}

#[test]
fn test_status_default_is_starting() {
    assert_eq!(SubagentStatus::default(), SubagentStatus::Starting);
}

#[test]
fn test_all_status_variants_distinct_display() {
    let variants = [
        SubagentStatus::Starting,
        SubagentStatus::Idle,
        SubagentStatus::Running,
        SubagentStatus::Error,
        SubagentStatus::Exited,
    ];
    let displays: Vec<String> = variants.iter().map(|v| format!("{}", v)).collect();
    let unique: std::collections::HashSet<&String> = displays.iter().collect();
    assert_eq!(displays.len(), unique.len());
}

// --- SubagentEntry ---

#[test]
fn test_new_entry_has_starting_status() {
    let entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 42);
    assert_eq!(entry.status, SubagentStatus::Starting);
    assert_eq!(entry.pid, 42);
    assert!(entry.last_tool.is_none());
    assert!(entry.last_error.is_none());
    assert!(entry.monitor_handle.is_none());
}

#[test]
fn test_entry_socket_path() {
    let entry = SubagentEntry::new(PathBuf::from("/run/quecto.sock"), 0);
    assert_eq!(entry.socket_path, PathBuf::from("/run/quecto.sock"));
}

// --- SubagentNotification (#523) ---

#[test]
fn test_completed_message_format() {
    let n = SubagentNotification::Completed {
        agent_id: "researcher".into(),
        summary: "All tests pass".into(),
    };
    let msg = n.to_message();
    assert!(msg.starts_with("[subagent]"));
    assert!(msg.contains("researcher"));
    assert!(msg.contains("completed"));
    assert!(msg.contains("All tests pass"));
}

#[test]
fn test_errored_message_format() {
    let n = SubagentNotification::Errored {
        agent_id: "linter".into(),
        error: "rate limit exceeded".into(),
    };
    let msg = n.to_message();
    assert!(msg.starts_with("[subagent]"));
    assert!(msg.contains("linter"));
    assert!(msg.contains("errored"));
    assert!(msg.contains("rate limit exceeded"));
}

#[test]
fn test_exited_message_format() {
    let n = SubagentNotification::Exited {
        agent_id: "formatter".into(),
    };
    let msg = n.to_message();
    assert!(msg.starts_with("[subagent]"));
    assert!(msg.contains("formatter"));
    assert!(msg.contains("exited"));
}

// --- extract_summary ---

#[test]
fn test_extract_summary_from_assistant_message() {
    let messages = serde_json::json!([
        {"role": "user", "content": "Do something"},
        {"role": "assistant", "content": "The analysis is complete"}
    ]);
    assert_eq!(extract_summary(&messages), "The analysis is complete");
}

#[test]
fn test_extract_summary_truncates_long_text() {
    let long = "x".repeat(300);
    let messages = serde_json::json!([
        {"role": "assistant", "content": long}
    ]);
    let summary = extract_summary(&messages);
    assert!(summary.len() <= 203); // 200 + "..."
    assert!(summary.ends_with("..."));
}

#[test]
fn test_extract_summary_empty_messages() {
    let messages = serde_json::json!([]);
    assert_eq!(extract_summary(&messages), "(no output)");
}

#[test]
fn test_extract_summary_no_assistant() {
    let messages = serde_json::json!([
        {"role": "tool", "content": "tool output"}
    ]);
    assert_eq!(extract_summary(&messages), "(no output)");
}

#[test]
fn test_extract_summary_truncates_multibyte_utf8() {
    // Each emoji is 4 bytes. 201 emojis = 804 bytes but 201 chars.
    let emojis = "🦀".repeat(201);
    let messages = serde_json::json!([{"role": "assistant", "content": emojis}]);
    let summary = extract_summary(&messages);
    assert!(summary.chars().count() <= 203); // 200 chars + "..."
    assert!(summary.ends_with("..."));
    // Should not panic on multi-byte boundary
}

#[test]
fn test_extract_summary_non_array() {
    let messages = serde_json::json!("not an array");
    assert_eq!(extract_summary(&messages), "(no output)");
}

#[test]
fn test_extract_summary_last_assistant() {
    let messages = serde_json::json!([
        {"role": "assistant", "content": "First response"},
        {"role": "user", "content": "Another question"},
        {"role": "assistant", "content": "Second response"}
    ]);
    assert_eq!(extract_summary(&messages), "Second response");
}

// --- notification channel ---

#[tokio::test]
async fn test_notification_channel_bounded() {
    let (tx, _rx) = new_notification_channel();
    for i in 0..NOTIFICATION_CHANNEL_CAPACITY {
        let n = SubagentNotification::Completed {
            agent_id: format!("bot-{}", i),
            summary: "done".into(),
        };
        assert!(
            tx.try_send(SequencedSubagentNotification::new(i as u64 + 1, n))
                .is_ok()
        );
    }
}

#[tokio::test]
async fn test_notification_drain() {
    let (tx, mut rx) = new_notification_channel();
    for i in 0..3 {
        let _ = tx
            .send(SequencedSubagentNotification::new(
                i as u64 + 1,
                SubagentNotification::Exited {
                    agent_id: format!("bot-{}", i),
                },
            ))
            .await;
    }
    drop(tx);
    let mut count = 0;
    while rx.recv().await.is_some() {
        count += 1;
    }
    assert_eq!(count, 3);
}

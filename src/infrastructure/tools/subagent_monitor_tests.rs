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

#[test]
fn test_truncate_string_char_boundary() {
    let s = "café résumé naïve";
    let max = 5; // within the "é" (2 bytes) after 5 ASCII chars "café " (6 bytes)
    let result = truncate_string(s, max);
    assert!(result.chars().count() <= max + 1); // allow ellipsis
    assert!(!result.is_empty());
}

#[test]
fn test_truncate_string_no_panic_on_multibyte() {
    // max_len falls inside a multi-byte char boundary; must not panic.
    let _ = truncate_string("é", 1);
    let _ = truncate_string("日本語", 3);
}

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

// --- apply_event: agent_error response ---

#[test]
fn test_agent_error_response_sets_error_and_last_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;
    apply_event(
        &mut entry,
        r#"{"type":"response","command":"agent_error","success":false,"error":"HTTP 404 model not found"}"#,
    );
    assert_eq!(entry.status, SubagentStatus::Error);
    assert_eq!(
        entry.last_error.as_deref(),
        Some("HTTP 404 model not found")
    );
}

#[test]
fn test_agent_start_clears_previous_agent_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Error;
    entry.last_error = Some("HTTP 404 model not found".to_string());
    apply_event(&mut entry, r#"{"type":"agent_start"}"#);
    assert_eq!(entry.status, SubagentStatus::Running);
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
async fn test_notify_on_agent_error_response() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"response","command":"agent_error","success":false,"error":"bad model"}"#;
    maybe_notify(Some(&tx), "worker", line);
    let notif = rx.try_recv().unwrap();
    match notif.notification {
        SubagentNotification::Errored {
            agent_id, error, ..
        } => {
            assert_eq!(agent_id, "worker");
            assert_eq!(error, "bad model");
        }
        _ => panic!("expected Errored"),
    }
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

#[test]
fn forward_child_workflow_event_retags_workflow_state() {
    let line = r#"{"type":"workflow_state","mode":"active","progress":{"done":1,"total":3}}"#;
    let out = forward_child_workflow_event(line, "child", Some("root")).expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["type"], "workflow_state");
    assert_eq!(v["agent_id"], "child");
    assert_eq!(v["parent_id"], "root");
}

#[test]
fn forward_child_workflow_event_ignores_non_workflow_lines() {
    assert!(forward_child_workflow_event(r#"{"type":"agent_end"}"#, "child", None).is_none());
    assert!(forward_child_workflow_event("not json", "child", None).is_none());
}

#[test]
fn forward_child_messages_appended_retags_with_child_id() {
    let line = r#"{"type":"subagent_messages_appended","agent_id":"","messages":[{"role":"assistant","content":"hi"}]}"#;
    let out = forward_child_messages_appended(line, "child", Some("root")).expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["type"], "subagent_messages_appended");
    assert_eq!(v["agent_id"], "child");
    assert_eq!(v["parent_id"], "root");
    assert_eq!(v["messages"][0]["role"], "assistant");
}

#[test]
fn forward_child_messages_appended_ignores_other_lines() {
    assert!(forward_child_messages_appended(r#"{"type":"agent_end"}"#, "child", None).is_none());
    assert!(forward_child_messages_appended("not json", "child", None).is_none());
}

// ─── #815: forward descendant subagent_state_changed up the monitor chain ───

#[test]
fn forward_child_state_changed_preserves_descendant_identity() {
    // A child's subagent_state_changed lists its OWN children (grandchildren of
    // the root). The forward MUST preserve each entry's real agentId/parentId —
    // it must NOT re-stamp them to the immediate child's id (#815).
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","status":"running","parentId":"child","pid":42}]}"#;
    let out = forward_child_state_changed(line).expect("state_changed line should be forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    let arr = v["subagents"].as_array().expect("subagents array");
    assert_eq!(arr.len(), 1);
    assert_eq!(
        arr[0]["agentId"], "grandchild",
        "descendant's real agent_id must be preserved (not re-stamped to the child)"
    );
    assert_eq!(
        arr[0]["parentId"], "child",
        "descendant's real parent_id must be preserved so it nests under its true parent"
    );
    assert_eq!(arr[0]["status"], "running");
}

#[test]
fn forward_child_state_changed_ignores_other_lines() {
    assert!(forward_child_state_changed(r#"{"type":"agent_end"}"#).is_none());
    assert!(forward_child_state_changed("not json").is_none());
}

#[test]
fn forward_child_state_changed_preserves_multiple_descendants() {
    // Two grandchildren under DIFFERENT parents in one event. A buggy impl that
    // stamps the whole list with a single id (or collapses it) would fail here;
    // each entry must keep its own agentId/parentId (#815).
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"gc-a","status":"running","parentId":"child-1"},{"agentId":"gc-b","status":"idle","parentId":"child-2"}]}"#;
    let out = forward_child_state_changed(line).expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let arr = v["subagents"].as_array().expect("subagents array");
    assert_eq!(arr.len(), 2);
    let find = |id: &str| {
        arr.iter()
            .find(|s| s["agentId"] == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    };
    assert_eq!(find("gc-a")["parentId"], "child-1");
    assert_eq!(find("gc-a")["status"], "running");
    assert_eq!(find("gc-b")["parentId"], "child-2");
    assert_eq!(find("gc-b")["status"], "idle");
}

#[test]
fn forward_child_state_changed_chains_already_forwarded_descendants() {
    // Multi-hop: a great-grandchild's entry (parentId is itself a grandchild,
    // NOT the immediate child reporting it) arrives at this monitor. Forwarding
    // must NOT be gated on `parentId == agent_id`, or great-grandchildren stay
    // invisible — the original bug class (#815).
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"great-grandchild","status":"running","parentId":"grandchild"}]}"#;
    let out = forward_child_state_changed(line).expect("forwarded regardless of parentId");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let arr = v["subagents"].as_array().expect("subagents array");
    assert_eq!(arr[0]["agentId"], "great-grandchild");
    assert_eq!(
        arr[0]["parentId"], "grandchild",
        "deep descendant identity must survive the hop unchanged"
    );
}

#[test]
fn handle_monitor_line_forwards_descendant_state_changed() {
    // Through the real handler: a child's subagent_state_changed (carrying a
    // grandchild) must be forwarded onto the parent/root broadcast so the TUI
    // panel can list it. Regression for #815: previously dropped by the
    // STATE_CHANGING_EVENTS pre-filter and never forwarded.
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","status":"running","parentId":"child","pid":7}]}"#;
    super::handle_monitor_line(line, "child", &registry, None, Some(&tx), Some("root"));
    let fwd: serde_json::Value = serde_json::from_str(
        &rx.try_recv()
            .expect("descendant state_changed must be forwarded to the parent stream"),
    )
    .unwrap();
    assert_eq!(fwd["type"], "subagent_state_changed");
    let arr = fwd["subagents"].as_array().expect("subagents array");
    assert_eq!(arr[0]["agentId"], "grandchild");
    assert_eq!(arr[0]["parentId"], "child");
}

#[test]
fn apply_event_parsed_records_workflow_snapshot() {
    let mut entry = test_entry();
    let value: serde_json::Value = serde_json::from_str(
        r#"{"type":"workflow_state","mode":"complete","progress":{"done":3,"total":3}}"#,
    )
    .unwrap();
    apply_event_parsed(&mut entry, &value);
    let wf = entry.workflow.expect("workflow snapshot recorded");
    assert_eq!(wf.mode, "complete");
    assert_eq!(wf.steps_completed, 3);
    assert_eq!(wf.steps_total, 3);
}

#[test]
fn handle_monitor_line_records_and_forwards_workflow_state() {
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let line = r#"{"type":"workflow_state","mode":"active","progress":{"done":2,"total":4}}"#;
    super::handle_monitor_line(line, "child", &registry, None, Some(&tx), Some("root"));
    // R-B3: the child's workflow snapshot is recorded on its entry.
    let wf = registry
        .lock()
        .unwrap()
        .get("child")
        .unwrap()
        .workflow
        .clone()
        .expect("workflow recorded");
    assert_eq!(wf.steps_completed, 2);
    assert_eq!(wf.steps_total, 4);
    // R-B2: the event is forwarded to the parent stream, re-tagged.
    let fwd: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
    assert_eq!(fwd["agent_id"], "child");
    assert_eq!(fwd["parent_id"], "root");
}

#[test]
fn handle_monitor_line_drops_oversized_line() {
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let big = format!(
        r#"{{"type":"workflow_state","x":"{}"}}"#,
        "z".repeat(2_000_000)
    );
    super::handle_monitor_line(&big, "child", &registry, None, None, None);
    assert!(
        registry
            .lock()
            .unwrap()
            .get("child")
            .unwrap()
            .workflow
            .is_none(),
        "oversized line must be dropped"
    );
}

#[test]
fn forward_child_workflow_event_with_no_parent_yields_null_parent() {
    let line = r#"{"type":"workflow_state","mode":"active"}"#;
    let out = forward_child_workflow_event(line, "child", None).expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["agent_id"], "child");
    assert!(v["parent_id"].is_null());
}

#[test]
fn apply_event_parsed_workflow_state_defaults_missing_progress() {
    let mut entry = test_entry();
    let value: serde_json::Value =
        serde_json::from_str(r#"{"type":"workflow_state","mode":"active"}"#).unwrap();
    apply_event_parsed(&mut entry, &value);
    let wf = entry.workflow.expect("workflow recorded");
    assert_eq!(wf.mode, "active");
    assert_eq!(wf.steps_completed, 0);
    assert_eq!(wf.steps_total, 0);
}

#[tokio::test]
async fn monitor_loop_forwards_child_workflow_state_to_broadcast() {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (btx, mut brx) = tokio::sync::broadcast::channel::<String>(8);
    let handle = spawn_monitor_task(
        "child".to_string(),
        sock.clone(),
        registry.clone(),
        None,
        Some(btx),
        Some("root".to_string()),
    );
    let (mut stream, _) = listener.accept().await.unwrap();
    stream
        .write_all(b"{\"type\":\"workflow_state\",\"mode\":\"active\",\"progress\":{\"done\":1,\"total\":2}}\n")
        .await
        .unwrap();
    let line = tokio::time::timeout(std::time::Duration::from_secs(3), brx.recv())
        .await
        .expect("monitor should forward within 3s")
        .expect("broadcast line");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["agent_id"], "child");
    assert_eq!(v["parent_id"], "root");
    // And the child's snapshot was recorded on the entry.
    assert!(
        registry
            .lock()
            .unwrap()
            .get("child")
            .unwrap()
            .workflow
            .is_some()
    );
    handle.abort();
}

#[test]
fn response_agent_error_sets_run_error_without_conflating_tool_errors() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;

    apply_event(
        &mut entry,
        r#"{"type":"response","command":"agent_error","success":false,"error":"provider rejected model"}"#,
    );

    assert_eq!(entry.status, SubagentStatus::Error);
    assert_eq!(entry.last_error.as_deref(), Some("provider rejected model"));
    assert_eq!(entry.run_error.as_deref(), Some("provider rejected model"));
}

#[test]
fn tool_error_does_not_set_run_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Running;

    apply_event(
        &mut entry,
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":true}"#,
    );

    assert_eq!(entry.status, SubagentStatus::Error);
    assert!(entry.last_error.as_ref().unwrap().contains("bash"));
    assert!(entry.run_error.is_none());
}

#[test]
fn agent_start_clears_previous_run_error() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Error;
    entry.last_error = Some("provider rejected model".to_string());
    entry.run_error = Some("provider rejected model".to_string());

    apply_event(&mut entry, r#"{"type":"agent_start"}"#);

    assert_eq!(entry.status, SubagentStatus::Running);
    assert!(entry.last_error.is_none());
    assert!(entry.run_error.is_none());
}

#[test]
fn agent_end_does_not_overwrite_run_error_with_idle() {
    let mut entry = test_entry();
    entry.status = SubagentStatus::Error;
    entry.run_error = Some("provider rejected model".to_string());

    apply_event(&mut entry, r#"{"type":"agent_end","messages":[]}"#);

    assert_eq!(entry.status, SubagentStatus::Error);
    assert_eq!(entry.run_error.as_deref(), Some("provider rejected model"));
}

#[tokio::test]
async fn response_agent_error_sends_errored_notification() {
    let (tx, mut rx) = super::super::subagent_registry::new_notification_channel();
    let line = r#"{"type":"response","command":"agent_error","success":false,"error":"bad model"}"#;

    maybe_notify(Some(&tx), "bot", line);

    let notification = rx.try_recv().expect("notification sent");
    match notification.notification {
        SubagentNotification::Errored { agent_id, error } => {
            assert_eq!(agent_id, "bot");
            assert_eq!(error, "bad model");
        }
        other => panic!("expected errored notification, got {other:?}"),
    }
}

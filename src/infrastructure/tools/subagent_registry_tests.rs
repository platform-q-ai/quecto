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

// --- consume_await_dedupe (#828) ---

#[test]
fn consume_await_dedupe_handles_none_registry() {
    // No registry at all: nothing to suppress.
    assert!(!consume_await_dedupe(&None, "anyone"));
}

#[test]
fn consume_await_dedupe_false_without_pending_flag() {
    let r = new_registry();
    r.lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
    assert!(!consume_await_dedupe(&Some(r), "bot"));
}

#[test]
fn consume_await_dedupe_consumes_pending_flag_once() {
    let r = new_registry();
    r.lock()
        .unwrap()
        .insert("bot".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
    mark_completion_consumed_by_await(&r, "bot");
    let reg = Some(r);
    // First check consumes the flag (suppress), second sees it cleared.
    assert!(consume_await_dedupe(&reg, "bot"));
    assert!(!consume_await_dedupe(&reg, "bot"));
}

// --- SubagentNotification (#523) ---

#[test]
fn test_completed_message_format() {
    let n = SubagentNotification::Completed {
        agent_id: "researcher".into(),
        summary: "All tests pass".into(),
    };
    let msg = n.to_message();
    assert!(msg.contains("researcher"));
    assert!(msg.contains("completed"));
    assert!(msg.contains("ready for inspection"));
    // The child's output is intentionally NOT repeated in the note.
    assert!(!msg.contains("All tests pass"));
}

#[test]
fn test_errored_message_format() {
    let n = SubagentNotification::Errored {
        agent_id: "linter".into(),
        error: "rate limit exceeded".into(),
    };
    let msg = n.to_message();
    assert!(msg.contains("linter"));
    assert!(msg.contains("failed"));
    assert!(msg.contains("rate limit exceeded"));
}

#[test]
fn test_exited_message_format() {
    let n = SubagentNotification::Exited {
        agent_id: "formatter".into(),
    };
    let msg = n.to_message();
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

// --- capped line reader (#795 security review) ---

#[tokio::test]
async fn read_line_capped_reads_lines_then_eof() {
    let data = b"first\nsecond\n";
    let mut reader = tokio::io::BufReader::new(&data[..]);
    assert_eq!(
        read_line_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some("first")
    );
    assert_eq!(
        read_line_capped(&mut reader, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some("second")
    );
    assert_eq!(read_line_capped(&mut reader, 1024).await.unwrap(), None);
}

#[tokio::test]
async fn read_line_capped_rejects_oversized_line() {
    let big = format!("{}\n", "x".repeat(100));
    let mut reader = tokio::io::BufReader::new(big.as_bytes());
    let err = read_line_capped(&mut reader, 16).await.unwrap_err();
    assert!(
        err.to_string().contains("exceeded size limit"),
        "expected size-limit error, got: {err}"
    );
}

// --- command/response matching (#831) ---

#[test]
fn stamp_request_id_injects_unique_id_into_object() {
    let (out, id) = stamp_request_id(r#"{"type":"get_messages_tail","count":5}"#);
    let id = id.expect("a JSON object command must get an id");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(id.as_str()));
    // The original fields are preserved.
    assert_eq!(
        v.get("type").and_then(|x| x.as_str()),
        Some("get_messages_tail")
    );
    assert_eq!(v.get("count").and_then(|x| x.as_u64()), Some(5));
    // Successive calls produce distinct ids.
    let (_, id2) = stamp_request_id(r#"{"type":"get_state"}"#);
    assert_ne!(Some(id), id2);
}

#[test]
fn stamp_request_id_overwrites_existing_id() {
    let (out, id) = stamp_request_id(r#"{"type":"get_state","id":"stale"}"#);
    let id = id.unwrap();
    assert_ne!(id, "stale");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v.get("id").and_then(|x| x.as_str()), Some(id.as_str()));
}

#[test]
fn stamp_request_id_none_for_non_object() {
    assert_eq!(stamp_request_id("not json"), ("not json".to_string(), None));
    assert_eq!(stamp_request_id("[1,2,3]"), ("[1,2,3]".to_string(), None));
}

#[tokio::test]
async fn command_reader_skips_connect_time_snapshot_and_returns_matching_reply() {
    // Reproduce #831: a BUSY child pushes an unsolicited connect-time
    // `get_messages` SNAPSHOT as the FIRST line, then the real reply. The
    // snapshot carries no `id`, while the real reply echoes the request id the
    // helper stamped — so the reader must return the latter (latest turns), not
    // the snapshot (the child's first message only). This also proves the fix
    // generalises to a `get_messages` request: the snapshot shares its command
    // string, yet id-correlation still skips it.
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        // Unsolicited connect-time snapshot (the child's FIRST message), no id.
        write_half
            .write_all(b"{\"type\":\"response\",\"command\":\"get_messages\",\"data\":[{\"content\":\"FIRST MESSAGE ONLY\"}]}\n")
            .await
            .unwrap();
        // Echo the request id the parent stamped, as the dispatch loop would.
        let mut lines = BufReader::new(read_half).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let req: serde_json::Value = serde_json::from_str(&line).unwrap();
        let id = req.get("id").and_then(|v| v.as_str()).unwrap();
        let reply = format!(
            "{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_messages_tail\",\"data\":[{{\"content\":\"LATEST TURNS\"}}]}}\n"
        );
        write_half.write_all(reply.as_bytes()).await.unwrap();
        // Keep the connection alive briefly so the reader can consume both lines.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let cmd = r#"{"type":"get_messages_tail","count":5}"#;
    let reply =
        send_subagent_uds_command_with_timeout(&sock, cmd, std::time::Duration::from_secs(3))
            .await
            .expect("reader should return a response");

    assert!(
        reply.contains("get_messages_tail") && reply.contains("LATEST TURNS"),
        "expected the get_messages_tail reply, got: {reply}"
    );
    assert!(
        !reply.contains("FIRST MESSAGE ONLY"),
        "must not return the connect-time snapshot, got: {reply}"
    );
    server.await.unwrap();
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

// Cascade-remove tests moved to `subagent_cascade_tests.rs` alongside the
// extracted `subagent_cascade` module (#831).

#[test]
fn snapshot_response_is_valid_for_uncounted_get_messages_and_get_state_only() {
    let messages_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_messages",
        "data": { "messages": [] }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &messages_snapshot,
        r#"{"type":"get_messages","count":1}"#
    ));

    let state_snapshot = serde_json::json!({
        "type": "response",
        "command": "get_state",
        "data": { "isStreaming": true, "messageCount": 2 }
    });
    assert!(subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_session_stats"}"#
    ));
}

#[test]
fn snapshot_response_rejects_invalid_or_mismatched_commands() {
    let state_snapshot = serde_json::json!({"type":"response","command":"get_state","data":{}});
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        "not-json"
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"count":1}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_state","agent_id":"child"}"#
    ));
    assert!(!subagent_snapshot::response_is_valid_answer(
        &state_snapshot,
        r#"{"type":"get_messages"}"#
    ));
}

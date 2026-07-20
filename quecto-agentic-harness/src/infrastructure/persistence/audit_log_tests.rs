use super::*;
use crate::domain::audit::AuditEvent;
use tempfile::TempDir;

#[tokio::test]
async fn creates_audit_directory_on_open() {
    let tmp = TempDir::new().unwrap();
    let base = tmp.path();
    assert!(!base.join("audit").exists());

    let _log = AuditLog::open(base, "test-session").await.unwrap();
    assert!(base.join("audit").exists());
}

#[tokio::test]
async fn writes_valid_jsonl_with_envelope() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

    log.emit(
        7,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "call_abc".into(),
            arguments: r#"{"command":"test"}"#.into(),
        },
    )
    .await
    .unwrap();

    let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);

    let val: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(val["session"], "cli:my-feature");
    assert_eq!(val["turn"], 7);
    assert_eq!(val["event"], "tool_call");
    assert_eq!(val["tool"], "bash");
    assert_eq!(val["call_id"], "call_abc");
    // ts should be ISO 8601
    let ts = val["ts"].as_str().unwrap();
    assert!(ts.ends_with('Z'));
    assert!(ts.contains('T'));
}

#[tokio::test]
async fn appends_multiple_events_in_order() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "multi").await.unwrap();

    log.emit(
        1,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
        },
    )
    .await
    .unwrap();
    log.emit(
        1,
        AuditEvent::ToolResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            is_error: false,
            content_tokens: 100,
            content_preview: "ok".into(),
        },
    )
    .await
    .unwrap();
    log.emit(
        2,
        AuditEvent::LlmTurnStart {
            input_tokens_estimate: 5000,
            message_count: 10,
        },
    )
    .await
    .unwrap();
    log.emit(
        2,
        AuditEvent::LlmTurnEnd {
            input_tokens: 5000,
            output_tokens: 500,
            stop_reason: "end_turn".into(),
            duration_ms: 2000,
        },
    )
    .await
    .unwrap();

    let path = AuditLog::file_path(tmp.path(), "multi");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 4);

    let v0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let v1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    let v2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    let v3: serde_json::Value = serde_json::from_str(lines[3]).unwrap();
    assert_eq!(v0["event"], "tool_call");
    assert_eq!(v1["event"], "tool_result");
    assert_eq!(v2["event"], "llm_turn_start");
    assert_eq!(v3["event"], "llm_turn_end");
}

#[tokio::test]
async fn flushes_on_every_write() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "flush-test").await.unwrap();

    log.emit(
        1,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
        },
    )
    .await
    .unwrap();

    // Read without closing the log — file should be readable
    let path = AuditLog::file_path(tmp.path(), "flush-test");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content.lines().count(), 1);

    // Emit another event
    log.emit(
        2,
        AuditEvent::ToolCall {
            tool: "read".into(),
            call_id: "c2".into(),
            arguments: "{}".into(),
        },
    )
    .await
    .unwrap();

    let content2 = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content2.lines().count(), 2);
}

#[tokio::test]
async fn sanitizes_session_key_for_filename() {
    let tmp = TempDir::new().unwrap();
    let _log = AuditLog::open(tmp.path(), "cli:my-feature").await.unwrap();

    let path = AuditLog::file_path(tmp.path(), "cli:my-feature");
    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        "cli_my-feature.jsonl"
    );
}

#[tokio::test]
async fn all_event_types_write_successfully() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "all-types").await.unwrap();

    let events = vec![
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "c1".into(),
            arguments: "{}".into(),
        },
        AuditEvent::ToolResult {
            call_id: "c1".into(),
            tool: "bash".into(),
            is_error: false,
            content_tokens: 10,
            content_preview: "ok".into(),
        },
        AuditEvent::LlmTurnStart {
            input_tokens_estimate: 1000,
            message_count: 5,
        },
        AuditEvent::LlmTurnEnd {
            input_tokens: 1000,
            output_tokens: 200,
            stop_reason: "end_turn".into(),
            duration_ms: 500,
        },
        AuditEvent::WorkflowStep {
            action: "check".into(),
            step_index: 1,
            step_key: "tests".into(),
            step_label: "Write tests".into(),
            template_id: "feature".into(),
        },
        AuditEvent::WorkflowTransition {
            from_mode: "selector".into(),
            to_mode: "active".into(),
            template_id: Some("feature".into()),
            issue: None,
        },
        AuditEvent::ContextPruned {
            messages_dropped: 5,
            tool_results_collapsed: 0,
            tokens_before: 100_000,
            tokens_after: 80_000,
            budget_unmet: false,
        },
        AuditEvent::SubagentSpawned {
            agent_id: "reviewer".into(),
            task_preview: "Review code".into(),
            system_preview: "You are a reviewer".into(),
        },
        AuditEvent::SubagentCmd {
            agent_id: "reviewer".into(),
            command: "status".into(),
        },
        AuditEvent::GuardBlocked {
            command_preview: "git push".into(),
            guard_message: "Not ready".into(),
            before_step_key: "commit".into(),
        },
        AuditEvent::Error {
            source: "provider".into(),
            tool: None,
            message: "rate limited".into(),
        },
    ];

    for (i, event) in events.into_iter().enumerate() {
        log.emit(i as u32 + 1, event).await.unwrap();
    }

    let path = AuditLog::file_path(tmp.path(), "all-types");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 11);

    // Verify each line is valid JSON with envelope
    for (i, line) in lines.iter().enumerate() {
        let val: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(val["ts"].is_string(), "line {} missing ts", i);
        assert_eq!(val["session"], "all-types");
        assert_eq!(val["turn"], i as u64 + 1);
        assert!(val["event"].is_string(), "line {} missing event", i);
    }
}

#[test]
fn open_sync_creates_sanitized_log_file_and_debug_redacts_path() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open_sync(tmp.path(), "cli:sync-session").unwrap();
    let path = AuditLog::file_path(tmp.path(), "cli:sync-session");

    assert!(path.exists(), "open_sync must create the append log file");
    assert_eq!(path.file_name().unwrap(), "cli_sync-session.jsonl");
    let debug = format!("{log:?}");
    assert!(
        debug.contains("cli:sync-session"),
        "debug should name the session: {debug}"
    );
    assert!(
        !debug.contains(tmp.path().to_str().unwrap()),
        "debug must not expose the filesystem path: {debug}"
    );
}

#[test]
fn unix_to_utc_handles_leap_year_boundaries_and_centuries() {
    assert_eq!(unix_to_utc(951_782_400), (2000, 2, 29, 0, 0, 0));
    assert_eq!(unix_to_utc(951_868_800), (2000, 3, 1, 0, 0, 0));
    assert!(is_leap(2000));
    assert!(!is_leap(1900));
    assert!(is_leap(2024));
    assert!(!is_leap(2025));
}

#[test]
fn unix_to_utc_epoch() {
    assert_eq!(unix_to_utc(0), (1970, 1, 1, 0, 0, 0));
}

#[test]
fn unix_to_utc_known_date() {
    // 2026-03-28T00:00:00Z = 1774656000
    let (y, m, d, h, mi, s) = unix_to_utc(1_774_656_000);
    assert_eq!((y, m, d, h, mi, s), (2026, 3, 28, 0, 0, 0));
}

#[test]
fn now_utc_iso8601_format() {
    let ts = now_utc_iso8601();
    assert!(ts.ends_with('Z'));
    assert!(ts.contains('T'));
    assert_eq!(ts.len(), 24); // "YYYY-MM-DDTHH:MM:SS.mmmZ"
}

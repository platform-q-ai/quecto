//! Contract tests for the `AuditSink` port.

use quecto::domain::audit::{AuditEvent, AuditSink};
use quecto::infrastructure::persistence::audit_log::AuditLog;
use std::sync::Arc;

fn tool_call(id: &str) -> AuditEvent {
    AuditEvent::ToolCall {
        tool: "bash".into(),
        call_id: id.into(),
        arguments: "{\"command\":\"ls\"}".into(),
    }
}

#[tokio::test]
async fn emit_appends_one_json_line_per_event() {
    let tmp = tempfile::tempdir().unwrap();
    let path = AuditLog::file_path(tmp.path(), "session-a");
    let sink: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-a").await.unwrap());
    sink.emit(0, tool_call("c1")).await.unwrap();
    sink.emit(1, tool_call("c2")).await.unwrap();
    // emit() guarantees a flush on return, so data is readable without waiting.
    let body = std::fs::read_to_string(&path).expect("audit log must exist");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "one line per emit, got {body:?}");
    for line in &lines {
        let _v: serde_json::Value =
            serde_json::from_str(line).expect("each line must be valid JSON");
    }
}

#[tokio::test]
async fn emit_persists_across_sink_reopens() {
    let tmp = tempfile::tempdir().unwrap();
    let path = AuditLog::file_path(tmp.path(), "session-dur");
    {
        let sink: Arc<dyn AuditSink> =
            Arc::new(AuditLog::open(tmp.path(), "session-dur").await.unwrap());
        sink.emit(0, tool_call("first")).await.unwrap();
    }
    {
        let sink: Arc<dyn AuditSink> =
            Arc::new(AuditLog::open(tmp.path(), "session-dur").await.unwrap());
        sink.emit(1, tool_call("second")).await.unwrap();
    }
    let body = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        body.lines().count(),
        2,
        "reopen must preserve prior events and append the new one; got:\n{body}"
    );
}

#[tokio::test]
async fn different_session_keys_produce_separate_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let path_a = AuditLog::file_path(tmp.path(), "session-a");
    let path_b = AuditLog::file_path(tmp.path(), "session-b");
    let a: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-a").await.unwrap());
    let b: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-b").await.unwrap());
    a.emit(0, tool_call("only-a")).await.unwrap();
    b.emit(0, tool_call("only-b")).await.unwrap();

    let a_log = std::fs::read_to_string(&path_a).unwrap();
    let b_log = std::fs::read_to_string(&path_b).unwrap();
    assert!(a_log.contains("only-a"));
    assert!(!a_log.contains("only-b"));
    assert!(b_log.contains("only-b"));
    assert!(!b_log.contains("only-a"));
}

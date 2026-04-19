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
    {
        let sink: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-a").await.unwrap());
        sink.emit(0, tool_call("c1")).await.unwrap();
        sink.emit(1, tool_call("c2")).await.unwrap();
        // tokio::fs::File's Drop does not guarantee an async flush of any
        // pending write through the spawn_blocking boundary. Yield so the
        // underlying write task gets to complete before we read.
        tokio::task::yield_now().await;
    }

    // Allow the OS to drain any pending buffered write after Drop.
    for _ in 0..10 {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        if body.lines().count() >= 2 {
            for line in body.lines() {
                let _v: serde_json::Value = serde_json::from_str(line)
                    .expect("each line must be valid JSON");
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    panic!("audit log did not reach 2 lines after drop; got:\n{body}");
}

#[tokio::test]
async fn emit_persists_across_sink_reopens() {
    let tmp = tempfile::tempdir().unwrap();
    let path = AuditLog::file_path(tmp.path(), "session-dur");
    {
        let sink: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-dur").await.unwrap());
        sink.emit(0, tool_call("first")).await.unwrap();
        tokio::task::yield_now().await;
    }
    {
        let sink: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-dur").await.unwrap());
        sink.emit(1, tool_call("second")).await.unwrap();
        tokio::task::yield_now().await;
    }
    for _ in 0..10 {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        if body.lines().count() >= 2 { return; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    panic!("reopen must preserve prior events and append the new one; got:\n{body}");
}

#[tokio::test]
async fn different_session_keys_produce_separate_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let path_a = AuditLog::file_path(tmp.path(), "session-a");
    let path_b = AuditLog::file_path(tmp.path(), "session-b");
    {
        let a: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-a").await.unwrap());
        let b: Arc<dyn AuditSink> = Arc::new(AuditLog::open(tmp.path(), "session-b").await.unwrap());
        a.emit(0, tool_call("only-a")).await.unwrap();
        b.emit(0, tool_call("only-b")).await.unwrap();
        tokio::task::yield_now().await;
    }
    let mut a_log = String::new();
    let mut b_log = String::new();
    for _ in 0..10 {
        a_log = std::fs::read_to_string(&path_a).unwrap_or_default();
        b_log = std::fs::read_to_string(&path_b).unwrap_or_default();
        if a_log.contains("only-a") && b_log.contains("only-b") { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(a_log.contains("only-a"), "session-a log missing content: {a_log}");
    assert!(!a_log.contains("only-b"));
    assert!(b_log.contains("only-b"), "session-b log missing content: {b_log}");
    assert!(!b_log.contains("only-a"));
}

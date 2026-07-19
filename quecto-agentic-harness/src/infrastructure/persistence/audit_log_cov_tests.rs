use super::*;
use crate::domain::audit::{AuditEvent, AuditSink};
use tempfile::TempDir;

#[tokio::test]
async fn open_sync_and_trait_emit_append_json_lines_to_sanitized_path() {
    let tmp = TempDir::new().unwrap();
    let session = "session/with spaces";
    let log = AuditLog::open_sync(tmp.path(), session).unwrap();
    let path = AuditLog::file_path(tmp.path(), session);
    // Unsafe characters push the key through hex-encoding, never raw.
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("key_") && name.ends_with(".jsonl"),
        "{name}"
    );
    assert!(!name.contains(' ') && !name.contains('/'));

    AuditSink::emit(
        &log,
        3,
        AuditEvent::ToolCall {
            tool: "bash".into(),
            call_id: "call-1".into(),
            arguments: "echo hi".into(),
        },
    )
    .await
    .unwrap();
    log.emit(
        4,
        AuditEvent::Error {
            source: "unit".into(),
            tool: Some("bash".into()),
            message: "boom".into(),
        },
    )
    .await
    .unwrap();

    let content = tokio::fs::read_to_string(&path).await.unwrap();
    let lines: Vec<_> = content.lines().collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["session"], session);
    assert_eq!(first["turn"], 3);
    assert_eq!(first["event"], "tool_call");
    assert!(first["ts"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test]
async fn async_open_creates_appendable_log() {
    let tmp = TempDir::new().unwrap();
    let log = AuditLog::open(tmp.path(), "async").await.unwrap();
    log.emit(
        1,
        AuditEvent::LlmTurnStart {
            input_tokens_estimate: 9,
            message_count: 2,
        },
    )
    .await
    .unwrap();
    let content = tokio::fs::read_to_string(AuditLog::file_path(tmp.path(), "async"))
        .await
        .unwrap();
    assert!(content.contains("llm_turn_start"));
    assert!(format!("{log:?}").contains("async"));
}

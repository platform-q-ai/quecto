//! Contract tests for the `SearchLog` port (#2136), over its JSONL adapter:
//! each record is one JSON line carrying the record's fields, and a log
//! that cannot be written never fails the caller.

use quecto::application::search::ports::{SearchLog, SearchRecord};
use quecto::infrastructure::search::search_log::JsonlSearchLog;

fn record() -> SearchRecord {
    SearchRecord {
        arguments: r#"{"pattern": "retry"}"#.into(),
        output: "content".into(),
        found: 4,
        incomplete: false,
        error: None,
        elapsed_ms: 7,
        ranking: None,
    }
}

#[test]
fn each_record_is_one_json_line_with_its_fields() {
    let base = tempfile::tempdir().unwrap();
    let log: Box<dyn SearchLog> = Box::new(JsonlSearchLog::new(base.path(), "session-a"));
    log.record(&record());
    log.record(&record());
    let dir = base.path().join("search-log");
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(file)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["found"], 4);
    assert_eq!(lines[0]["arguments"]["pattern"], "retry");
    assert_eq!(lines[0]["session"], "session-a");
}

#[test]
fn an_unwritable_log_never_fails_the_caller() {
    let base = tempfile::tempdir().unwrap();
    std::fs::write(base.path().join("search-log"), "not a directory").unwrap();
    let log: Box<dyn SearchLog> = Box::new(JsonlSearchLog::new(base.path(), "session-a"));
    log.record(&record());
}

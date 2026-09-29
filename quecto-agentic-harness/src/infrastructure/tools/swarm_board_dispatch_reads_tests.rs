use serde_json::json;

use super::tests::{captured, running};
use super::{TELEMETRY_TARGET, call};

/// The read models (#2277) record at DEBUG, as every member polls them
/// (the lifecycle watch loop); the summaries `create`, `_join` and
/// `_bootstrap` answer with record at INFO, as the writes they are. No
/// argument text reaches the log.
#[test]
fn read_models_record_at_debug_without_argument_text() {
    let secret = "sk-ant-api03-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let log = captured(9, |handles| {
        call(handles, "parent", "_bootstrap", json!([1, "s", null])).unwrap();
        running(handles);
        call(
            handles,
            "parent",
            "task_create",
            json!([format!("req {secret}"), "t", ["tests pass"]]),
        )
        .unwrap();
        let summary = call(handles, "parent", "summary", json!([])).unwrap();
        let cursor = summary["event_cursor"].clone();
        let unchanged = call(handles, "parent", "summary", json!([cursor])).unwrap();
        assert_eq!(unchanged["unchanged"], json!(true));
        call(handles, "parent", "events", json!({"after": 0, "limit": 2})).unwrap();
        call(handles, "parent", "tasks", json!([])).unwrap();
        call(handles, "parent", "task", json!([1])).unwrap();
        call(handles, "parent", "summary", json!([secret])).unwrap_err();
        call(handles, "parent", "_join", json!([null, 1, "s", null])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .collect();
    let expected = [
        (" INFO ", "_bootstrap", "ok"),
        (" INFO ", "create_run", "ok"),
        (" INFO ", "task_create", "ok"),
        (" DEBUG ", "summary", "ok"),
        (" DEBUG ", "summary", "ok"),
        (" DEBUG ", "events", "ok"),
        (" DEBUG ", "tasks", "ok"),
        (" DEBUG ", "task", "ok"),
        (" DEBUG ", "summary", "refused"),
        (" INFO ", "_join", "ok"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            "member=\"parent\"".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(
        records[4].contains("decision=\"unchanged\""),
        "{}",
        records[4]
    );
    assert!(records[3].contains("decision=\"full\""), "{}", records[3]);
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
}

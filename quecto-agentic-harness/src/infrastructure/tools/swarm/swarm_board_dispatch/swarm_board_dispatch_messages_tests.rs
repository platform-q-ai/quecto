use serde_json::json;

use super::super::tests::{board, captured, running};
use super::super::{TELEMETRY_TARGET, call};

/// Each message call leaves one record with its decision (`inbox` at
/// DEBUG, the rest at INFO) and no argument text: a secret-shaped body,
/// revision or request id never reaches the log.
#[test]
fn message_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let log = captured(10, |handles| {
        running(handles);
        let body = format!("body {secret}");
        let request = format!("req-{secret}");
        let send = json!([request, "parent", body, format!("rev {secret}")]);
        call(handles, "parent", "send", send.clone()).unwrap();
        call(handles, "parent", "send", send).unwrap();
        call(handles, "parent", "inbox", json!([])).unwrap();
        call(handles, "parent", "ack", json!([1])).unwrap();
        call(handles, "parent", "ack", json!([1])).unwrap();
        call(handles, "parent", "withdraw", json!([1])).unwrap_err();
        call(handles, "parent", "send", json!(["two", "parent", body])).unwrap();
        call(handles, "parent", "withdraw", json!([2])).unwrap();
        call(handles, "parent", "withdraw", json!([2])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        (" INFO ", "send", "ok", "sent"),
        (" INFO ", "send", "ok", "replayed"),
        (" DEBUG ", "inbox", "ok", "read"),
        (" INFO ", "ack", "ok", "consumed"),
        (" INFO ", "ack", "ok", "unchanged"),
        (" INFO ", "withdraw", "refused", "none"),
        (" INFO ", "send", "ok", "sent"),
        (" INFO ", "withdraw", "ok", "withdrawn"),
        (" INFO ", "withdraw", "ok", "unchanged"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "member=\"parent\"".to_owned(),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
    assert!(!log.contains("body "), "argument text never logged: {log}");
    assert!(!log.contains("rev "), "argument text never logged: {log}");
}

/// `send` answers its receipt, `inbox` each row as `dict(row)` in table
/// order, and `withdraw` and `ack` answer `{message_id, changed}` (#2394).
#[test]
fn message_methods_render_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let receipt = call(&handles, "parent", "send", json!(["a", "parent", "hi"])).unwrap();
    assert_eq!(receipt, json!({"id": 1, "status": "accepted"}));
    let inbox = call(
        &handles,
        "parent",
        "inbox",
        json!({"include_consumed": true}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&inbox).unwrap(),
        r#"[{"id":1,"sender":"parent","recipient":"parent","body":"hi","status":"accepted","revision":null,"supersedes":null,"superseded_by":null}]"#
    );
    assert_eq!(
        serde_json::to_string(&call(&handles, "parent", "ack", json!([1])).unwrap()).unwrap(),
        r#"{"message_id":1,"changed":true}"#
    );
    assert_eq!(
        call(&handles, "parent", "withdraw", json!([1]))
            .unwrap_err()
            .message(),
        "message 1 is already consumed"
    );
}

/// Each wake call (#2276) leaves one DEBUG record (both run as Python's
/// read-only operation) with its decision and no argument text; the
/// answers are Python's shapes.
#[test]
fn wake_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let log = captured(6, |handles| {
        running(handles);
        let body = format!("body {secret}");
        call(handles, "parent", "send", json!(["a", "parent", body])).unwrap();
        let batch = call(handles, "parent", "_notifications", json!([true])).unwrap();
        assert_eq!(batch, json!({"members": [], "generation": 2}));
        assert_eq!(
            call(
                handles,
                "parent",
                "_notifications",
                json!({"with_generation": secret})
            )
            .unwrap(),
            json!({"members": [], "generation": 2})
        );
        assert_eq!(
            call(handles, "parent", "_accept_wake", json!([2])).unwrap(),
            json!(false)
        );
        call(handles, "parent", "_accept_wake", json!([secret])).unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(2)
        .collect();
    let expected = [
        ("_notifications", "ok", "quiet"),
        ("_notifications", "ok", "quiet"),
        ("_accept_wake", "ok", "not_woken"),
        ("_accept_wake", "refused", "none"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            " DEBUG ".to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "member=\"parent\"".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("body "), "argument text never logged: {log}");
}

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
/// order, and `withdraw` and `ack` answer `null`.
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
        call(&handles, "parent", "ack", json!([1])).unwrap(),
        json!(null)
    );
    assert_eq!(
        call(&handles, "parent", "withdraw", json!([1]))
            .unwrap_err()
            .message(),
        "message 1 is already consumed"
    );
}

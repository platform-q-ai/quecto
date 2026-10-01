use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, running};
use crate::domain::swarm::{BoardError, RefusalKind};
use crate::infrastructure::tools::swarm_board_dispatch::{SwarmBoardHandles, call};

/// A running run of three coordinated by `parent`, with `worker` live and
/// holding the claim of task 1; its token.
fn claimed(handles: &SwarmBoardHandles) -> Value {
    running(handles);
    call(handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r", 7, "t", null]),
    )
    .unwrap();
    call(
        handles,
        "worker",
        "task_create",
        json!(["r1", "first", ["tests pass"]]),
    )
    .unwrap();
    call(handles, "worker", "claim", json!([1])).unwrap()["token"].clone()
}

/// Each method binds Python's signature and answers the task's dict as it
/// now stands (#2394); the task moves through blocked, claimed, submitted
/// and completed.
#[test]
fn submission_methods_answer_the_task_and_move_it() {
    let (_dir, handles) = board(1_000.0);
    let token = claimed(&handles);
    let status = || call(&handles, "worker", "task_raw", json!([1])).unwrap()["status"].clone();
    let evidence = json!([{"artifact": "report", "revision": "R1"}]);
    for (member, method, args, after) in [
        ("worker", "block", json!([1, token, "waiting"]), "blocked"),
        (
            "worker",
            "unblock",
            json!({"task_id": 1, "token": token, "reason": "resolved"}),
            "claimed",
        ),
        ("worker", "submit", json!([1, token, evidence]), "submitted"),
        (
            "parent",
            "verify_task",
            json!({"revision": "R1", "token": token, "task_id": 1}),
            "completed",
        ),
    ] {
        let answer = call(&handles, member, method, args).unwrap();
        assert_eq!(answer["status"], json!(after), "{method}");
        assert_eq!(status(), json!(after), "{method}");
    }
    for (method, message) in [
        ("block", "block: missing required argument reason"),
        ("unblock", "unblock: missing required argument reason"),
        ("submit", "submit: missing required argument evidence"),
        (
            "verify_task",
            "verify_task: missing required argument revision",
        ),
    ] {
        assert_eq!(
            call(&handles, "worker", method, json!([1, token])).unwrap_err(),
            BoardError::new(RefusalKind::Calling, message)
        );
    }
}

/// Each call leaves one INFO record with its decision (`unchanged` for a
/// call that found the task already as asked) and no argument text: a
/// secret-shaped token, reason, evidence or revision never reaches the
/// log.
#[test]
fn submission_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
    let log = captured(14, |handles| {
        let token = claimed(handles);
        let reason = format!("see {secret}");
        let evidence = json!([{"artifact": format!("art {secret}"), "revision": secret}]);
        for (member, method, args) in [
            ("worker", "block", json!([1, token, reason])),
            ("worker", "block", json!([1, token, reason])),
            ("worker", "unblock", json!([1, token, reason])),
            ("worker", "unblock", json!([1, token, reason])),
            ("worker", "submit", json!([1, token, evidence])),
            ("worker", "submit", json!([1, token, evidence])),
            ("parent", "verify_task", json!([1, token, secret])),
            ("parent", "verify_task", json!([1, token, secret])),
        ] {
            call(handles, member, method, args).unwrap();
        }
        call(handles, "worker", "block", json!([1, secret, reason])).unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(5)
        .collect();
    let expected = [
        ("worker", "block", "ok", "blocked"),
        ("worker", "block", "ok", "unchanged"),
        ("worker", "unblock", "ok", "unblocked"),
        ("worker", "unblock", "ok", "unchanged"),
        ("worker", "submit", "ok", "submitted"),
        ("worker", "submit", "ok", "unchanged"),
        ("parent", "verify_task", "ok", "verified"),
        ("parent", "verify_task", "ok", "unchanged"),
        ("worker", "block", "refused", "none"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (member, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            " INFO ".to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            format!("member=\"{member}\""),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
    assert!(!log.contains("see "), "argument text never logged: {log}");
    assert!(!log.contains("art "), "argument text never logged: {log}");
}

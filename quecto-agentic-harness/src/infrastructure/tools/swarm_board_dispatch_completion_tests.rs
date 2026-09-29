use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, running};
use crate::domain::swarm::BoardError;
use crate::infrastructure::tools::swarm_board_dispatch::call;

/// Each completion method answers `None`, as Python's do; a completed run
/// is paused holding `succeeded` until the supervisor closes it.
#[test]
fn completion_methods_answer_none_and_complete_holds_success() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    for (method, args) in [
        ("evidence", json!(["t", "ci.log", "R1", "command", true])),
        ("complete", json!(["R1"])),
    ] {
        assert_eq!(call(&handles, "parent", method, args).unwrap(), Value::Null);
    }
    let status = call(&handles, "parent", "_control_status", json!([])).unwrap();
    assert_eq!(
        (&status["status"], &status["outcome"], &status["reason"]),
        (
            &json!("paused"),
            &json!("succeeded"),
            &json!("completed at R1")
        )
    );
}

/// Each argument binds Python's signature, by position or by name.
#[test]
fn completion_methods_bind_pythons_signatures() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    for (method, args, message) in [
        (
            "complete",
            json!([]),
            "complete: missing required argument revision",
        ),
        (
            "revalidate_task",
            json!([1, "R1"]),
            "revalidate_task: missing required argument evidence",
        ),
        (
            "amend",
            json!({"goal": "g", "constraints": [], "criteria": []}),
            "amend: missing required argument reason",
        ),
        (
            "evidence",
            json!(["t", "a", "R1", "command", true, 1]),
            "evidence: takes 5 arguments, 6 given",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap_err(),
            BoardError::new(message)
        );
    }
    let amended = call(
        &handles,
        "parent",
        "amend",
        json!({
            "reason": "scope agreed",
            "criteria": [{"id": "t", "kind": "review", "description": "d"}],
            "constraints": [],
            "goal": "new goal",
        }),
    );
    assert_eq!(amended.unwrap(), Value::Null);
}

/// Each completion call leaves one INFO record with its decision, and no
/// argument text: a secret-shaped goal, reason or artifact never reaches
/// the log.
#[test]
fn completion_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let text = format!("see {secret}");
    let log = captured(6, |handles| {
        running(handles);
        let criteria = json!([{"id": "t", "kind": "command", "description": "d"}]);
        for (method, args) in [
            ("amend", json!([text, [text], criteria, text])),
            ("evidence", json!(["t", text, "R1", "command", true])),
            ("evidence", json!(["t", text, "R1", "command", true])),
            ("complete", json!(["R1"])),
        ] {
            call(handles, "parent", method, args).unwrap();
        }
        call(
            handles,
            "parent",
            "revalidate_task",
            json!([1, "R1", [{"artifact": text, "revision": "R1"}]]),
        )
        .unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        ("amend", "ok", "amended"),
        ("evidence", "ok", "recorded"),
        ("evidence", "ok", "unchanged"),
        ("complete", "ok", "completed"),
        ("revalidate_task", "refused", "none"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            " INFO ".to_owned(),
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
    assert!(!log.contains("see "), "argument text never logged: {log}");
}

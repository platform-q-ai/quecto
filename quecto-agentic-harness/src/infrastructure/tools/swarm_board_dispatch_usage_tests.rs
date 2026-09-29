use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, captured_on, running};
use crate::domain::swarm::BoardError;
use crate::infrastructure::tools::swarm_board_dispatch::call;

fn failed(request_id: &str) -> Value {
    json!({"request_id": request_id, "instrumented_attempts": 1, "outcome": "failed"})
}

/// `_request_admission` answers Python's dict, key for key and in order:
/// status, coordinator, deadline, members (each as `dict(row)`), outcome
/// and the control generation.
#[test]
fn request_admission_answers_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let admission = call(&handles, "parent", "_request_admission", json!([])).unwrap();
    assert_eq!(
        serde_json::to_string(&admission).unwrap(),
        r#"{"status":"running","coordinator":"parent","deadline":4600.0,"members":[{"id":"parent","reservation":"00000000000000000000000000000002","status":"live","pid":null,"started":null,"socket":null,"launcher":null}],"outcome":null,"control_generation":0}"#
    );
}

/// `usage_budget` answers the report with the budget in Python's
/// insertion order; `_record_request` answers the control receipt.
#[test]
fn usage_budget_and_record_request_answer_pythons_shapes() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let report = call(&handles, "parent", "usage_budget", json!([100])).unwrap();
    assert_eq!(
        serde_json::to_string(&report["budget"]).unwrap(),
        r#"{"token_limit":100,"strict_unknown":true,"warned":false}"#
    );
    assert_eq!(report["totals"]["requests"], json!(0));
    let receipt = call(&handles, "parent", "_record_request", json!([failed("r1")])).unwrap();
    assert_eq!(
        serde_json::to_string(&receipt).unwrap(),
        r#"{"status":"paused","outcome":"budget-exhausted","reason":"observed usage budget or unavailable measurement","generation":5,"budget":{"token_limit":100,"strict_unknown":true,"warned":true,"observed_tokens":0,"unknown_usage_requests":1},"resume_blockers":["raise or disable the token budget (swarm_control usage_budget) before resuming"]}"#
    );
}

/// Each argument binds Python's signature: `strict_unknown` defaults to
/// `True`, and a missing or extra argument is calling syntax.
#[test]
fn usage_methods_bind_pythons_signatures() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    for (method, args, message) in [
        (
            "usage_budget",
            json!([]),
            "usage_budget: missing required argument token_limit",
        ),
        (
            "usage_budget",
            json!([1, true, 3]),
            "usage_budget: takes 2 arguments, 3 given",
        ),
        (
            "_record_request",
            json!({}),
            "_record_request: missing required argument record",
        ),
        (
            "_request_admission",
            json!([1]),
            "_request_admission: takes 0 arguments, 1 given",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap_err(),
            BoardError::new(message)
        );
    }
    let report = call(
        &handles,
        "parent",
        "usage_budget",
        json!({"token_limit": null, "strict_unknown": false}),
    )
    .unwrap();
    assert_eq!(report["budget"]["strict_unknown"], json!(false));
}

/// Each usage call leaves one record: INFO for `usage_budget` and
/// `_record_request`, DEBUG for the admission read, each with the decision
/// it took (the budget's effect first); a secret-shaped request id or
/// record field never reaches the log.
#[test]
fn usage_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let log = captured(9, |handles| {
        running(handles);
        let mut record = failed(secret);
        record["error_class"] = json!(format!("see {secret}"));
        record["context_input_tokens"] = json!(40);
        record["output_tokens"] = json!(0);
        for (method, args) in [
            ("usage_budget", json!([100, false])),
            ("usage_budget", json!([100, false])),
            ("_request_admission", json!([])),
            ("_record_request", json!([record.clone()])),
            ("_record_request", json!([record])),
            ("usage_budget", json!([40, false])),
            ("_request_admission", json!([])),
        ] {
            call(handles, "parent", method, args).unwrap();
        }
        call(handles, "parent", "_record_request", json!([{}])).unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        (" INFO ", "usage_budget", "ok", "configured"),
        (" INFO ", "usage_budget", "ok", "unchanged"),
        ("DEBUG ", "_request_admission", "ok", "read"),
        (" INFO ", "_record_request", "ok", "recorded"),
        (" INFO ", "_record_request", "ok", "redelivered"),
        (" INFO ", "usage_budget", "ok", "paused"),
        ("DEBUG ", "_request_admission", "ok", "read"),
        (" INFO ", "_record_request", "refused", "none"),
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
    assert!(!log.contains("see "), "argument text never logged: {log}");
}

/// The admission read records at DEBUG, but one whose budget warns or
/// pauses the run (a budget another writer set, here written straight into
/// the file) records at INFO, so every budget pause is visible at the
/// default level; a redelivery that rewrites the record is `replaced`.
#[test]
fn an_admission_read_that_warns_or_pauses_records_at_info() {
    let known = json!({"request_id": "k", "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": 45, "output_tokens": 5,
        "runtime": {"process_instance_id": "p", "executable_sha256": "abc"}});
    let log = captured_on(6, |handles, database| {
        running(handles);
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        let budget = |limit: u64| {
            let written = rusqlite::Connection::open(database)
                .unwrap()
                .execute(
                    "UPDATE usage_budget SET payload=? WHERE id=1",
                    [format!(
                        r#"{{"token_limit": {limit}, "strict_unknown": false, "warned": false}}"#
                    )],
                )
                .unwrap();
            assert_eq!(written, 1, "the budget row");
        };
        call(handles, "parent", "usage_budget", json!([1_000, false])).unwrap();
        budget(60);
        call(handles, "parent", "_request_admission", json!([])).unwrap();
        budget(40);
        call(handles, "parent", "_request_admission", json!([])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        (" INFO ", "_record_request", "recorded"),
        (" INFO ", "_record_request", "replaced"),
        (" INFO ", "usage_budget", "configured"),
        (" INFO ", "_request_admission", "warned"),
        (" INFO ", "_request_admission", "paused"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, decision)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("decision=\"{decision}\""),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
}

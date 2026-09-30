use serde_json::{Value, json};

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, captured_on, running};
use crate::domain::swarm::{BoardError, RefusalKind};
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
            json!(["model", 1]),
            "_request_admission: takes 1 arguments, 2 given",
        ),
        (
            "_request_admission",
            json!({"gates": "tool"}),
            "_request_admission: unexpected argument gates",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap_err(),
            BoardError::new(RefusalKind::Calling, message)
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
        ("DEBUG ", "_request_admission", "ok", "model_gate"),
        (" INFO ", "_record_request", "ok", "recorded"),
        (" INFO ", "_record_request", "ok", "redelivered"),
        (" INFO ", "usage_budget", "ok", "paused"),
        ("DEBUG ", "_request_admission", "ok", "model_gate"),
        (" INFO ", "_record_request", "refused", "none"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            level.to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "commit_us=".to_owned(),
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

/// Only the usage methods raise their level for the budget's `paused` or
/// `warned`: another method's decision of the same name keeps its own
/// level, and a usage method's other decisions keep the method's.
#[test]
fn only_a_usage_method_is_raised_by_the_budgets_decision() {
    use super::super::Level;
    use super::super::method::Method;
    for decision in ["paused", "warned"] {
        assert_eq!(
            Method::RequestAdmission.served_level(decision),
            Level::Mutation
        );
        for method in [Method::Status, Method::Snapshot, Method::UsageReport] {
            assert_eq!(method.served_level(decision), Level::Read, "{method:?}");
        }
    }
    for gate in ["model_gate", "retry_gate", "tool_gate"] {
        assert_eq!(Method::RequestAdmission.served_level(gate), Level::Read);
    }
    assert_eq!(Method::Pause.served_level("paused"), Level::Mutation);
}

/// #2339: the admission read takes the gate that reads it, `model` (the
/// default, so a call without one is the first send of a model request),
/// `retry` or `tool`, and records it as the decision (`model_gate`,
/// `retry_gate`, `tool_gate`) with the same answer; any other gate is
/// refused as invalid, and its text never reaches the log.
#[test]
fn the_admission_read_records_the_gate_that_read_it() {
    let secret = "sk-ant-api03-GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG";
    let log = captured(10, |handles| {
        running(handles);
        let answer = call(handles, "parent", "_request_admission", json!([])).unwrap();
        for args in [
            json!(["model"]),
            json!(["retry"]),
            json!(["tool"]),
            json!({"gate": "tool"}),
        ] {
            assert_eq!(
                call(handles, "parent", "_request_admission", args.clone()).unwrap(),
                answer,
                "{args}"
            );
        }
        for gate in [json!(secret), json!(null), json!(1), json!("Tool")] {
            assert_eq!(
                call(handles, "parent", "_request_admission", json!([gate])).unwrap_err(),
                BoardError::new(
                    RefusalKind::Invalid,
                    "_request_admission: gate must be one of model, retry, tool"
                ),
            );
        }
    });
    let decisions: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET) && line.contains("_request_admission"))
        .map(|line| {
            ["model_gate", "retry_gate", "tool_gate", "none"]
                .into_iter()
                .find(|decision| line.contains(&format!("decision=\"{decision}\"")))
                .unwrap_or("?")
        })
        .collect();
    assert_eq!(
        decisions,
        [
            "model_gate",
            "model_gate",
            "retry_gate",
            "tool_gate",
            "tool_gate",
            "none",
            "none",
            "none",
            "none"
        ],
        "{log}"
    );
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
}

/// #2339 review N4: a retry or tool gate's read in which the budget warns
/// or pauses the run records the budget's `warned` or `paused` at INFO in
/// place of the gate, as the model gate's does.
#[test]
fn a_retry_or_tool_gate_read_that_warns_or_pauses_records_the_budget() {
    let known = json!({"request_id": "k", "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": 45, "output_tokens": 5,
        "runtime": {"process_instance_id": "p", "executable_sha256": "abc"}});
    let log = captured_on(5, |handles, database| {
        running(handles);
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        call(handles, "parent", "usage_budget", json!([1_000, false])).unwrap();
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
        budget(60);
        call(handles, "parent", "_request_admission", json!(["retry"])).unwrap();
        budget(40);
        call(handles, "parent", "_request_admission", json!(["tool"])).unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET) && line.contains("_request_admission"))
        .collect();
    let expected = [(" INFO ", "warned"), (" INFO ", "paused")];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, decision)) in records.iter().zip(expected) {
        for field in [level.to_owned(), format!("decision=\"{decision}\"")] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
}

/// #2340: recording a request does the same work at the end of a long run
/// as at its start, counted rather than timed (a disk's `fsync` makes a
/// timing bound flaky; the opt-in `record_request_latency` example times
/// it). Each call adds the one row it inserted to the ledger sums the
/// process kept, and runs no whole-ledger SQL; a control receipt adds
/// none. Before the fix each call read the whole usage report twice.
#[test]
fn recording_a_request_does_not_grow_with_the_ledger() {
    use super::super::tests::board_and_repository;
    use crate::infrastructure::persistence::swarm_board::LedgerWork;
    const CALLS: usize = 1_000;
    let (_dir, handles, repository) = board_and_repository(1_000.0);
    running(&handles);
    let work_of = |method: &str, args: Value| {
        let before = repository.ledger_work();
        call(&handles, "parent", method, args).unwrap();
        let after = repository.ledger_work();
        LedgerWork {
            rows_scanned: after.rows_scanned - before.rows_scanned,
            whole_ledger_reads: after.whole_ledger_reads - before.whole_ledger_reads,
        }
    };
    let one_row = LedgerWork {
        rows_scanned: 1,
        whole_ledger_reads: 0,
    };
    for index in 0..CALLS {
        let record = json!([{"request_id": format!("r{index}"), "instrumented_attempts": 1,
            "outcome": "succeeded", "context_input_tokens": 3, "output_tokens": 1}]);
        assert_eq!(
            work_of("_record_request", record),
            one_row,
            "request {index}"
        );
        if index % 100 == 99 {
            assert_eq!(
                work_of("_control_status", json!([])),
                LedgerWork::default(),
                "the receipt after request {index}"
            );
        }
    }
    let status = call(&handles, "parent", "_control_status", json!([])).unwrap();
    assert_eq!(status["budget"]["observed_tokens"], json!(4 * CALLS));
}

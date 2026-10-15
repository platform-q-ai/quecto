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

/// `_record_request` answers the control receipt in Python's key order,
/// the run still running, its budget the default no board writes over any
/// more: an unmeasured request pauses nothing (owner decision,
/// 2026-10-14). `usage_budget` is no method.
#[test]
fn record_request_answers_pythons_shape_and_usage_budget_is_gone() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let receipt = call(&handles, "parent", "_record_request", json!([failed("r1")])).unwrap();
    assert_eq!(
        serde_json::to_string(&receipt).unwrap(),
        r#"{"status":"running","outcome":null,"reason":null,"generation":0,"budget":{"token_limit":null,"strict_unknown":false,"warned":false,"observed_tokens":0,"unknown_usage_requests":1},"resume_blockers":[]}"#
    );
    for args in [json!([100]), json!([null, false])] {
        let refused = call(&handles, "parent", "usage_budget", args).unwrap_err();
        assert_eq!(refused.kind(), RefusalKind::Calling, "{refused}");
    }
}

/// Each argument binds Python's signature: `strict_unknown` defaults to
/// `True`, and a missing or extra argument is calling syntax.
#[test]
fn usage_methods_bind_pythons_signatures() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    for (method, args, message) in [
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
}

/// Each usage call leaves one record: INFO for `_record_request`, DEBUG
/// for the admission read, each with the decision it took; a
/// secret-shaped request id or record field never reaches the log.
#[test]
fn usage_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let log = captured(6, |handles| {
        running(handles);
        let mut record = failed(secret);
        record["error_class"] = json!(format!("see {secret}"));
        record["context_input_tokens"] = json!(40);
        record["output_tokens"] = json!(0);
        for (method, args) in [
            ("_request_admission", json!([])),
            ("_record_request", json!([record.clone()])),
            ("_record_request", json!([record])),
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
        ("DEBUG ", "_request_admission", "ok", "model_gate"),
        (" INFO ", "_record_request", "ok", "recorded"),
        (" INFO ", "_record_request", "ok", "redelivered"),
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

/// The admission read records at DEBUG with its gate, whatever budget
/// another writer stored (here written straight into the file, below and
/// then far below the usage): no budget warns or pauses the run (owner
/// decision, 2026-10-14). A redelivery that rewrites the record is
/// `replaced`.
#[test]
fn an_admission_read_under_a_spent_budget_records_its_gate_at_debug() {
    let known = json!({"request_id": "k", "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": 45, "output_tokens": 5,
        "runtime": {"process_instance_id": "p", "executable_sha256": "abc"}});
    let log = captured_on(5, |handles, database| {
        running(handles);
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        // A budget row an older board stored: nothing writes one now.
        let budget = |limit: u64| {
            let written = rusqlite::Connection::open(database)
                .unwrap()
                .execute(
                    "INSERT OR REPLACE INTO usage_budget VALUES(1, ?)",
                    [format!(
                        r#"{{"token_limit": {limit}, "strict_unknown": false, "warned": false}}"#
                    )],
                )
                .unwrap();
            assert_eq!(written, 1, "the budget row");
        };
        budget(60);
        let admitted = call(handles, "parent", "_request_admission", json!([])).unwrap();
        assert_eq!(admitted["status"], json!("running"));
        budget(40);
        let admitted = call(handles, "parent", "_request_admission", json!([])).unwrap();
        assert_eq!(admitted["status"], json!("running"));
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        (" INFO ", "_record_request", "recorded"),
        (" INFO ", "_record_request", "replaced"),
        ("DEBUG ", "_request_admission", "model_gate"),
        ("DEBUG ", "_request_admission", "model_gate"),
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

/// No method raises its level for a decision named `paused` or `warned`
/// as a budget's: the usage methods never decide either (owner decision,
/// 2026-10-14), and `pause`'s own `paused` keeps its level.
#[test]
fn no_usage_method_is_raised_by_a_budget_decision() {
    use super::super::Level;
    use super::super::method::Method;
    for decision in ["paused", "warned"] {
        assert_eq!(Method::RequestAdmission.served_level(decision), Level::Read);
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

/// #2339 review N4: a retry or tool gate's read records its gate, at
/// DEBUG, under a stored budget the usage has passed: no budget warns or
/// pauses the run (owner decision, 2026-10-14).
#[test]
fn a_retry_or_tool_gate_read_under_a_spent_budget_records_its_gate() {
    let known = json!({"request_id": "k", "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": 45, "output_tokens": 5,
        "runtime": {"process_instance_id": "p", "executable_sha256": "abc"}});
    let log = captured_on(4, |handles, database| {
        running(handles);
        call(handles, "parent", "_record_request", json!([known.clone()])).unwrap();
        // A budget row an older board stored: nothing writes one now.
        let budget = |limit: u64| {
            let written = rusqlite::Connection::open(database)
                .unwrap()
                .execute(
                    "INSERT OR REPLACE INTO usage_budget VALUES(1, ?)",
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
    let expected = [("DEBUG ", "retry_gate"), ("DEBUG ", "tool_gate")];
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
    use super::super::tests::{board_and_repository, metered_board_and_repository};
    use crate::infrastructure::persistence::swarm_board::LedgerWork;
    const CALLS: usize = 1_000;
    // The plain repository, and (the event log on) a metered repository
    // per call, which shares the sums the plain one keeps.
    for (dir_handles_repository, path) in [
        (board_and_repository(1_000.0), "plain"),
        (metered_board_and_repository(1_000.0), "metered"),
    ] {
        let (_dir, handles, repository) = dir_handles_repository;
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
                "{path}: request {index}"
            );
            if index % 100 == 99 {
                assert_eq!(
                    work_of("_control_status", json!([])),
                    LedgerWork::default(),
                    "{path}: the receipt after request {index}"
                );
            }
        }
        let status = call(&handles, "parent", "_control_status", json!([])).unwrap();
        assert_eq!(
            status["budget"]["observed_tokens"],
            json!(4 * CALLS),
            "{path}"
        );
    }
}

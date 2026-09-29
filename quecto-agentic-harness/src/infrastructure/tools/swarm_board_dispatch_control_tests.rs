use serde_json::json;

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, running};
use crate::domain::swarm::{BoardError, RefusalKind, RunStatus};
use crate::infrastructure::tools::swarm_board_dispatch::call;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// The receipt is Python's `_receipt` dict, key for key and in order: the
/// run's status, outcome and reason, the control generation, the budget
/// with the observed totals, and the resume blockers.
#[test]
fn control_methods_answer_the_receipt_in_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let stopped = call(
        &handles,
        "parent",
        "stop",
        json!(["blocked", "needs input"]),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&stopped).unwrap(),
        r#"{"status":"paused","outcome":"blocked","reason":"needs input","generation":3,"budget":{"token_limit":null,"strict_unknown":false,"warned":false,"observed_tokens":0,"unknown_usage_requests":0},"resume_blockers":[]}"#
    );
    let status = call(&handles, "worker", "_control_status", json!([])).unwrap_err();
    assert_eq!(
        status,
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    assert_eq!(
        call(&handles, "parent", "_control_status", json!([])).unwrap(),
        stopped,
        "the status reads the receipt the stop answered"
    );
    let closed = call(&handles, "parent", "_close", json!({})).unwrap();
    assert_eq!(closed["status"], json!("blocked"));
    assert_eq!(
        call(&handles, "parent", "resume", json!([])).unwrap_err(),
        BoardError::new(
            RefusalKind::SupervisorOnly,
            "a paused run is resumed only by the supervisor outside the swarm \
             (agent_cmd swarm_control resume); members cannot resume it"
        )
    );
}

/// `control_receipt_shape_decodes_as_run_control_receipt`: the
/// supervisor's `swarm_control` decodes the Rust receipt field by field.
#[test]
fn control_receipt_shape_decodes_as_run_control_receipt() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let paused = call(&handles, "parent", "pause", json!(["hold"])).unwrap();
    let decoded = SwarmContext::decode_control_receipt(paused, false).unwrap();
    assert_eq!(decoded.status, RunStatus::Paused);
    assert_eq!((decoded.outcome, decoded.reason), (None, None));
    assert_eq!(decoded.generation, 2);
    let budget = decoded.budget.expect("the receipt carries the budget");
    assert_eq!(
        (budget.token_limit, budget.strict_unknown, budget.warned),
        (None, false, false)
    );
    assert_eq!(
        (budget.observed_tokens, budget.unknown_usage_requests),
        (0, 0)
    );
    assert!(
        decoded.resume_blockers.is_empty(),
        "{:?}",
        decoded.resume_blockers
    );
    let ended = call(&handles, "parent", "_resume_external", json!([])).unwrap();
    let ended = SwarmContext::decode_control_receipt(ended, true).unwrap();
    assert_eq!(ended.status, RunStatus::Running);
    let stopped = call(&handles, "parent", "stop", json!(["failed", "broken"])).unwrap();
    let stopped = SwarmContext::decode_control_receipt(stopped, false).unwrap();
    assert_eq!(
        (stopped.status, stopped.outcome, stopped.reason.as_deref()),
        (RunStatus::Paused, Some(RunStatus::Failed), Some("broken"))
    );
}

/// `usage_report` on a board with no requests: the default budget and
/// zero aggregates under the SQL's aliases.
#[test]
fn usage_report_answers_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let report = call(&handles, "parent", "usage_report", json!([])).unwrap();
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        r#"{"budget":{"token_limit":null,"strict_unknown":false,"warned":false},"totals":{"requests":0,"observed_tokens":0,"unknown_usage_requests":0,"attempts":0,"reported_input_tokens":0,"reported_output_tokens":0,"reported_cache_read_tokens":0,"reported_cache_write_tokens":0,"cache_read_known_requests":0,"cache_write_known_requests":0},"members":[],"recent_requests":[]}"#
    );
}

/// Each argument binds Python's signature.
#[test]
fn control_methods_bind_pythons_signatures() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    for (method, args, message) in [
        (
            "pause",
            json!([]),
            "pause: missing required argument reason",
        ),
        (
            "stop",
            json!(["blocked"]),
            "stop: missing required argument reason",
        ),
        (
            "_extend_deadline",
            json!({}),
            "_extend_deadline: missing required argument seconds",
        ),
        ("_close", json!([1]), "_close: takes 0 arguments, 1 given"),
        (
            "_control_status",
            json!({"x": 1}),
            "_control_status: unexpected argument x",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap_err(),
            BoardError::new(RefusalKind::Calling, message)
        );
    }
    let extended = call(
        &handles,
        "parent",
        "_extend_deadline",
        json!({"seconds": 60}),
    )
    .unwrap();
    assert_eq!(extended["status"], json!("running"));
}

/// Each control call leaves one record: INFO for a change (with its
/// decision, `unchanged` for a no-op), DEBUG for the two reads, and no
/// argument text: a secret-shaped reason never reaches the log.
#[test]
fn control_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
    let reason = format!("see {secret}");
    let log = captured(12, |handles| {
        running(handles);
        for (method, args) in [
            ("pause", json!([reason])),
            ("pause", json!([reason])),
            ("_control_status", json!([])),
            ("_resume_external", json!([])),
            ("_resume_external", json!([])),
            ("_extend_deadline", json!([60])),
            ("stop", json!(["blocked", reason])),
            ("stop", json!(["blocked", reason])),
            ("_close", json!([])),
            ("usage_report", json!([])),
        ] {
            call(handles, "parent", method, args).unwrap();
        }
        call(handles, "parent", "resume", json!([])).unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .skip(1)
        .collect();
    let expected = [
        (" INFO ", "pause", "ok", "paused"),
        (" INFO ", "pause", "ok", "unchanged"),
        ("DEBUG ", "_control_status", "ok", "read"),
        (" INFO ", "_resume_external", "ok", "resumed"),
        (" INFO ", "_resume_external", "ok", "unchanged"),
        (" INFO ", "_extend_deadline", "ok", "extended"),
        (" INFO ", "stop", "ok", "stopped"),
        (" INFO ", "stop", "ok", "unchanged"),
        (" INFO ", "_close", "ok", "closed"),
        ("DEBUG ", "usage_report", "ok", "read"),
        (" INFO ", "resume", "refused", "none"),
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

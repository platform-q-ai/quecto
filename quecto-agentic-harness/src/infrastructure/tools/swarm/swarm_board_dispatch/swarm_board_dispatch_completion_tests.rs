use serde_json::json;

use super::super::TELEMETRY_TARGET;
use super::super::tests::{board, captured, running};
use crate::domain::swarm::{
    events::telemetry::RefusalKind, value_objects::board_error::BoardError,
};
use crate::infrastructure::tools::swarm::swarm_board_dispatch::call;

/// `evidence` answers the evidence as recorded and `complete` the run as it
/// now stands (#2394, where Python answered `None`); a completed run
/// is paused holding `succeeded` until the supervisor closes it.
#[test]
fn completion_methods_answer_what_they_changed_and_complete_holds_success() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let recorded = call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    assert_eq!(recorded["accepted"], json!(true), "{recorded}");
    let completed = call(&handles, "parent", "complete", json!(["R1"])).unwrap();
    let status = call(&handles, "parent", "_control_status", json!([])).unwrap();
    for field in ["status", "outcome", "reason"] {
        assert_eq!(completed[field], status[field], "{field}: {completed}");
    }
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
            BoardError::new(RefusalKind::Calling, message)
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
    assert_eq!(
        amended.unwrap(),
        json!({"goal": "new goal", "constraints": [],
               "criteria": [{"id": "t", "kind": "review", "description": "d"}]})
    );
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

/// #2481 S4c follow-up: the board a supervising session reads as a closed
/// run's coordinator stops (`_board`, what the final-member teardown
/// keeps) reads as the view the project board folds its run back from:
/// the run's end, each item's title, state, owner and evidence, and the
/// tokens it spent.
#[test]
fn a_closed_runs_board_reads_as_the_view_its_board_run_is_folded_from() {
    use super::super::{CallOrigin, call_as};
    use crate::application::project_board::dto::{SwarmEnd, SwarmEvidence, SwarmItemState};
    use crate::infrastructure::tools::agents::coordinator_launch::swarm_run_view;
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    call(&handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
    call(
        &handles,
        "parent",
        "_activate",
        json!(["worker", "r", 7, "t", null]),
    )
    .unwrap();
    call(
        &handles,
        "worker",
        "task_create",
        json!(["r1", "api: Build the api", ["tests pass"]]),
    )
    .unwrap();
    let token = call(&handles, "worker", "claim", json!([1])).unwrap()["token"].clone();
    let evidence = json!([{"artifact": "docs/api.md", "revision": "R1"}]);
    call(&handles, "worker", "submit", json!([1, token, evidence])).unwrap();
    call(
        &handles,
        "parent",
        "verify_task",
        json!({"revision": "R1", "token": token, "task_id": 1}),
    )
    .unwrap();
    call(
        &handles,
        "parent",
        "evidence",
        json!(["t", "ci.log", "R1", "command", true]),
    )
    .unwrap();
    call(&handles, "parent", "complete", json!(["R1"])).unwrap();
    let read = || {
        call_as(
            &handles,
            "supervisor",
            "_board",
            json!([]),
            CallOrigin::Supervisor,
        )
        .unwrap()
    };
    assert_eq!(
        swarm_run_view(&read()),
        None,
        "paused holding its outcome, it has not ended"
    );
    call(&handles, "parent", "_close", json!([])).unwrap();
    let view = swarm_run_view(&read()).expect("a closed run's board is a view");
    assert_eq!(view.end, SwarmEnd::Succeeded);
    let [item] = view.items.as_slice() else {
        panic!("one item: {view:?}");
    };
    assert_eq!(
        (item.title.as_str(), item.state),
        ("api: Build the api", SwarmItemState::Completed)
    );
    assert_eq!(
        item.evidence,
        [SwarmEvidence {
            artifact: "docs/api.md".into(),
            revision: "R1".into(),
        }]
    );
    assert_eq!(item.owner.as_deref(), Some("worker"), "its holder stays");
    assert_eq!(view.observed_tokens, 0, "no usage was reported");
}

//! Differential scenarios (#2273, epic #2265): completion, task
//! revalidation, contract amendment and the criterion evidence success
//! needs, on the Rust board against the Python board's answers frozen in
//! its golden fixtures (#2283), compared after every step by result,
//! refusal text and logical database dump. Ported from
//! the deleted Python suite `tests/swarm_helpers_test.py`, plus the loosely typed arguments Python
//! accepts (epic P3).
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{HOUR, at, snapshot};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_golden, run_rust, rust_answers, sql, step,
};
use crate::swarm_board_diff_runs::{NOW, assert_changed_answer_refused, harness_self_test_tamper};

/// `WorkbenchBehavior.setUp`: `parent` creates `ship feature` with a
/// command and a review criterion, and `worker` is admitted and live.
fn with_worker(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    let mut steps = vec![
        step(
            "parent",
            "create_run",
            json!([
                "ship feature",
                ["clean architecture"],
                [
                    {"id": "tests", "kind": "command", "description": "acceptance tests pass"},
                    {"id": "review", "kind": "review", "description": "independent review"}
                ],
                3,
                NOW + HOUR
            ]),
            NOW,
        ),
        at(1.0, "parent", "_admit", json!(["worker", "reservation-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "reservation-w", 12_345, "start-w", "/tmp/w.sock"]),
        ),
    ];
    steps.extend(more);
    steps
}

fn evidence(offset: f64, member: &str, criterion: &str, kind: &str, passed: Value) -> Step {
    at(
        offset,
        member,
        "evidence",
        json!([criterion, "final-check", "R2", kind, passed]),
    )
}

/// `worker` creates, claims, submits (at `revision`) and `parent`
/// verifies task `id`, from `offset`.
fn verified(offset: f64, id: i64, dependencies: Value, revision: &str) -> Vec<Step> {
    let token = format!("{:032x}", id + 2);
    vec![
        at(
            offset,
            "worker",
            "task_create",
            json!([
                format!("t{id}"),
                format!("t{id}"),
                ["tests pass"],
                dependencies
            ]),
        ),
        at(offset + 0.1, "worker", "claim", json!([id])),
        at(
            offset + 0.2,
            "worker",
            "submit",
            json!([id, token, [{"artifact": format!("t{id}-tests"), "revision": revision}]]),
        ),
        at(
            offset + 0.3,
            "parent",
            "verify_task",
            json!([id, token, revision]),
        ),
    ]
}

/// `test_dependent_tasks_can_be_revalidated_at_final_revision`: a task
/// verified at an earlier revision makes completion stale until the
/// coordinator revalidates it at the final one; a worker cannot, and
/// evidence at another revision is refused.
#[test]
fn dependent_tasks_can_be_revalidated_at_final_revision() {
    let mut setup = verified(3.0, 1, json!([]), "R1");
    setup.extend(verified(4.0, 2, json!([1]), "R2"));
    setup.extend([
        evidence(5.0, "parent", "tests", "command", json!(true)),
        evidence(5.1, "parent", "review", "review", json!(true)),
    ]);
    let refused = [
        at(6.0, "parent", "complete", json!(["R2"])),
        at(
            7.0,
            "worker",
            "revalidate_task",
            json!([1, "R2", [{"artifact": "a-rerun-at-R2", "revision": "R2"}]]),
        ),
        at(
            8.0,
            "parent",
            "revalidate_task",
            json!([1, "R2", [{"artifact": "old", "revision": "R1"}]]),
        ),
    ];
    let granted = [
        at(
            9.0,
            "parent",
            "revalidate_task",
            json!({"task_id": "1", "revision": "R2", "evidence": [{"revision": "R2", "artifact": "a-rerun-at-R2", "note": "é"}]}),
        ),
        at(10.0, "parent", "complete", json!({"revision": "R2"})),
        snapshot(11.0),
    ];
    let held = at(12.0, "parent", "complete", json!(["R2"]));
    let steps: Vec<Step> = setup
        .iter()
        .chain(&refused)
        .chain(&granted)
        .chain([&held])
        .cloned()
        .collect();
    run_golden(&with_worker(steps));
    // Every step but the refused ones is granted, and the run is held.
    let alone: Vec<Step> = setup.into_iter().chain(granted).chain([held]).collect();
    assert_eq!(
        run_rust(&with_worker(alone)),
        Outcome::Refused(
            "run is paused (succeeded: completed at R2); no new work permitted".to_owned()
        )
    );
}

/// `test_completion_holds_success_until_the_supervisor_closes_it` with a
/// real `complete`: nothing to close before it, success held (members
/// live) until the supervisor closes it, and then terminal.
#[test]
fn completion_holds_success_until_the_supervisor_closes_it() {
    run_golden(&held_success());
}

/// The steps of [`completion_holds_success_until_the_supervisor_closes_it`].
fn held_success() -> Vec<Step> {
    with_worker([
        evidence(3.0, "parent", "tests", "command", json!(true)),
        evidence(3.1, "parent", "review", "review", json!(true)),
        at(4.0, "parent", "_close", json!([])),
        at(
            5.0,
            "parent",
            "pause",
            json!(["a plain pause holds nothing to close"]),
        ),
        at(6.0, "parent", "_close", json!([])),
        at(7.0, "parent", "complete", json!(["R2"])),
        at(8.0, "parent", "_resume_external", json!([])),
        at(9.0, "parent", "complete", json!(["R2"])),
        snapshot(10.0),
        at(11.0, "parent", "_control_status", json!([])),
        at(12.0, "parent", "_close", json!([])),
        snapshot(13.0),
        at(
            14.0,
            "worker",
            "task_create",
            json!(["after", "after close", ["ok"]]),
        ),
        at(15.0, "parent", "_resume_external", json!([])),
    ])
}

/// Completion's refusals in Python's order: the revision, then accepted
/// evidence of each criterion's kind at it (a worker's proposal and a
/// non-`True` pass are not accepted), then settled work and reservations.
#[test]
fn completion_refuses_each_unsatisfied_requirement() {
    run_golden(&with_worker([
        at(3.0, "parent", "complete", json!([" "])),
        at(3.1, "parent", "complete", json!([5])),
        at(3.2, "parent", "complete", json!(["R2"])),
        evidence(4.0, "worker", "tests", "command", json!(true)),
        evidence(4.1, "parent", "review", "review", json!(1)),
        at(4.2, "parent", "complete", json!(["R2"])),
        evidence(5.0, "parent", "tests", "command", json!(true)),
        evidence(5.1, "parent", "review", "review", json!(true)),
        evidence(5.2, "parent", "review", "review", json!(true)),
        at(
            6.0,
            "worker",
            "task_create",
            json!(["open", "open", ["ok"]]),
        ),
        at(6.1, "parent", "complete", json!(["R2"])),
        at(6.2, "worker", "complete", json!(["R2"])),
        sql("DELETE FROM tasks; INSERT INTO files VALUES('src/a.rs',1,'worker','c','t')"),
        at(7.0, "parent", "complete", json!(["R2"])),
        sql("DELETE FROM files"),
        at(8.0, "parent", "complete", json!(["R1"])),
        at(9.0, "parent", "complete", json!(["R2"])),
        snapshot(10.0),
    ]));
}

/// `evidence` refuses an unknown criterion or another kind and bounds the
/// artifact and revision first; its criterion binds as Python binds it.
#[test]
fn evidence_arguments_are_checked_as_python_checks_them() {
    run_golden(&with_worker([
        at(
            3.0,
            "parent",
            "evidence",
            json!(["missing", "a", "R1", "command", true]),
        ),
        at(
            3.1,
            "parent",
            "evidence",
            json!(["tests", "a", "R1", "review", true]),
        ),
        at(
            3.2,
            "parent",
            "evidence",
            json!(["tests", "x".repeat(2_049), "R1", "command", true]),
        ),
        at(
            3.3,
            "parent",
            "evidence",
            json!(["tests", "a", "x".repeat(257), "command", true]),
        ),
        at(
            3.4,
            "parent",
            "evidence",
            json!(["tests", " ", "R1", "command", true]),
        ),
        at(
            3.5,
            "parent",
            "evidence",
            json!([["tests"], "a", "R1", "command", true]),
        ),
        at(
            3.6,
            "parent",
            "evidence",
            json!(["tests", "a", "R1", "command", "yes"]),
        ),
        at(
            3.7,
            "parent",
            "evidence",
            json!({"criterion": "tests", "artifact": "a", "revision": "R1", "kind": "command", "passed": true}),
        ),
        at(
            3.8,
            "parent",
            "evidence",
            json!(["tests", "a", "R1", "command", true]),
        ),
        at(
            3.9,
            "stranger",
            "evidence",
            json!(["tests", "a", "R1", "command", true]),
        ),
        at(4.0, "parent", "stop", json!(["blocked", "hold"])),
        at(
            4.1,
            "parent",
            "evidence",
            json!(["tests", "a", "R1", "command", true]),
        ),
    ]));
}

/// `test_amendment_preserves_the_entire_original_contract` and
/// `test_goal_is_durable_and_workers_cannot_amend_it`: only the
/// coordinator amends (a worker's bad criteria meet the coordinator check
/// first), every evidence row goes, and the event holds both contracts.
#[test]
fn amendment_preserves_the_entire_original_contract() {
    run_golden(&amended());
}

/// The steps of [`amendment_preserves_the_entire_original_contract`]: the
/// coordinator's `amend` is at `NOW + 5`.
fn amended() -> Vec<Step> {
    let changed = json!([{"id": "tests", "kind": "command", "description": "replacement test"}]);
    with_worker([
        evidence(3.0, "parent", "tests", "command", json!(true)),
        evidence(3.1, "worker", "review", "review", json!(false)),
        at(
            4.0,
            "worker",
            "amend",
            json!(["wrong", [], [], "silent change"]),
        ),
        at(
            5.0,
            "parent",
            "amend",
            json!([
                "ship feature",
                ["replacement constraint"],
                changed,
                "approved change"
            ]),
        ),
        snapshot(6.0),
        at(
            7.0,
            "parent",
            "amend",
            json!({"goal": "new goal", "constraints": {"not": "a list"}, "criteria": [{"id": "tests", "kind": "command", "description": "pass", "extra": [1.5, "é"]}], "reason": "scope agreed"}),
        ),
        at(8.0, "worker", "complete", json!(["R1"])),
    ])
}

/// #2394 round-1 review M1/L2: `evidence`, `amend` and `complete`'s
/// changed answers are checked against the board, column by column.
#[test]
fn harness_self_test_checks_completion_answers() {
    let steps = held_success();
    for (field, tampered) in [
        ("criterion", json!("review")),
        ("artifact", json!("bogus")),
        ("accepted", json!(false)),
        ("actor", json!("worker")),
    ] {
        let difference = harness_self_test_tamper(&steps, "evidence", 3.0, |answer| {
            answer.insert(field.to_owned(), tampered.clone());
        });
        assert_changed_answer_refused(difference, &format!("column {field} is not the board's"));
    }
    let completed = steps
        .iter()
        .zip(rust_answers(&steps))
        .find(|(step, answer)| step.method == "complete" && matches!(answer, Outcome::Ok(_)))
        .map(|(step, _)| step.now - NOW)
        .expect("a granted complete");
    for (field, tampered) in [
        ("reason", json!("completed at bogus")),
        ("status", json!("running")),
        ("outcome", json!("failed")),
    ] {
        let difference = harness_self_test_tamper(&steps, "complete", completed, |answer| {
            answer.insert(field.to_owned(), tampered.clone());
        });
        assert_changed_answer_refused(difference, &format!("column {field} is not the board's"));
    }
    let steps = amended();
    for (field, tampered) in [
        ("goal", json!("bogus")),
        ("constraints", json!(["bogus"])),
        ("criteria", json!([])),
    ] {
        let difference = harness_self_test_tamper(&steps, "amend", 5.0, |answer| {
            answer.insert(field.to_owned(), tampered.clone());
        });
        assert_changed_answer_refused(difference, &format!("column {field} is not the board's"));
    }
}

/// `amend` bounds the goal, the reason and the encoded constraints before
/// the gate and checks the criteria inside it, in Python's order.
#[test]
fn amendment_arguments_are_checked_as_python_checks_them() {
    let good = json!([{"id": "t", "kind": "review", "description": "d"}]);
    run_golden(&with_worker([
        at(3.0, "parent", "amend", json!([" ", [], good, 5])),
        at(3.1, "parent", "amend", json!(["g", [], good, null])),
        at(
            3.2,
            "parent",
            "amend",
            json!(["g", ["x".repeat(8_200)], good, "r"]),
        ),
        at(3.3, "parent", "amend", json!(["g", [], [], "r"])),
        at(3.4, "parent", "amend", json!(["g", [], "criteria", "r"])),
        at(
            3.5,
            "parent",
            "amend",
            json!(["g", [], [{"id": "t", "kind": "manual", "description": "d"}], "r"]),
        ),
        at(
            3.6,
            "parent",
            "amend",
            json!(["g", [], [good[0], good[0]], "r"]),
        ),
        at(
            3.7,
            "parent",
            "amend",
            json!(["g", [], [{"id": "t", "kind": "review", "description": "x".repeat(16_400)}], "r"]),
        ),
        at(
            3.8,
            "parent",
            "amend",
            json!(["g", [], [{"id": "t", "kind": "review", "description": "d", "pad": "x".repeat(16_400)}], "r"]),
        ),
        at(3.9, "stranger", "amend", json!(["g", [], good, "r"])),
        at(4.0, "parent", "pause", json!(["hold"])),
        at(4.1, "parent", "amend", json!(["g", [], good, "r"])),
    ]));
}

/// `revalidate_task` bounds the encoded evidence first, then needs the
/// coordinator, a known completed task, a revision and new evidence at it.
#[test]
fn revalidation_arguments_are_checked_as_python_checks_them() {
    let mut steps = verified(3.0, 1, json!([]), "R1");
    steps.extend([
        at(
            4.0,
            "worker",
            "task_create",
            json!(["open", "open", ["ok"]]),
        ),
        at(
            5.0,
            "parent",
            "revalidate_task",
            json!([1, "R2", [{"artifact": "x".repeat(8_200), "revision": "R2"}]]),
        ),
        at(
            5.1,
            "parent",
            "revalidate_task",
            json!([9, "R2", [{"artifact": "a", "revision": "R2"}]]),
        ),
        at(
            5.2,
            "parent",
            "revalidate_task",
            json!([2, "R2", [{"artifact": "a", "revision": "R2"}]]),
        ),
        at(
            5.3,
            "parent",
            "revalidate_task",
            json!([1, 2, [{"artifact": "a", "revision": 2}]]),
        ),
        at(5.4, "parent", "revalidate_task", json!([1, "R2", []])),
        at(
            5.5,
            "parent",
            "revalidate_task",
            json!([1, "R2", {"artifact": "a"}]),
        ),
        at(5.6, "parent", "revalidate_task", json!([1, "R2", ["a"]])),
        at(
            5.7,
            "parent",
            "revalidate_task",
            json!([1, "R2", [{"artifact": " ", "revision": "R2"}]]),
        ),
        at(
            5.8,
            "parent",
            "revalidate_task",
            json!([true, "R2", [{"artifact": "a", "revision": "R2"}]]),
        ),
        at(
            5.9,
            "parent",
            "revalidate_task",
            json!([1.0, "R3", [{"artifact": "b", "revision": "R3", "n": 1e16}]]),
        ),
        at(6.0, "parent", "task_raw", json!([1])),
    ]);
    run_golden(&with_worker(steps));
}

/// Every completion op needs a running run (`authorize(active=True)`, the
/// #2316 paused-run table's sibling; the #2319 mutation report's
/// `revalidate_task` survivor): on a paused run each is refused with
/// Python's own text, and the board is left as it was. Each op is one the
/// unpaused board grants, so only the run's status refuses it: task 1 is
/// completed at R2, and the parent accepted both criteria there.
#[test]
fn every_completion_op_is_refused_on_a_paused_run() {
    let mut setup = verified(3.0, 1, json!([]), "R2");
    setup.extend([
        evidence(5.0, "parent", "tests", "command", json!(true)),
        evidence(5.1, "parent", "review", "review", json!(true)),
    ]);
    let setup = with_worker(setup);
    let criteria = json!([{"id": "tests", "kind": "command", "description": "d"}]);
    let ops = [
        ("parent", "complete", json!(["R2"])),
        (
            "parent",
            "revalidate_task",
            json!([1, "R3", [{"artifact": "rerun", "revision": "R3"}]]),
        ),
        ("parent", "amend", json!(["g", [], criteria, "why"])),
        (
            "worker",
            "evidence",
            json!(["tests", "proposal", "R2", "command", false]),
        ),
        (
            "parent",
            "evidence",
            json!(["review", "other", "R2", "review", true]),
        ),
    ];
    let paused = sql("UPDATE run SET status='paused'");
    let refusal = "run is paused; no new work permitted";
    for (member, method, args) in ops {
        let op = at(9.0, member, method, args);
        let granted = [setup.clone(), vec![op.clone()]].concat();
        assert!(
            matches!(run_rust(&granted), Outcome::Ok(_)),
            "{member} {method} is granted on the running run"
        );
        let steps = [setup.clone(), vec![paused.clone(), op, snapshot(10.0)]].concat();
        run_golden(&steps);
        let outcome = run_rust(&steps[..steps.len() - 1]);
        assert!(
            matches!(&outcome, Outcome::Refused(text) if text == refusal),
            "{member} {method}: {outcome:?}"
        );
    }
}

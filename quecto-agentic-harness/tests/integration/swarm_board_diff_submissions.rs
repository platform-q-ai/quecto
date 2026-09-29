//! Differential scenarios (#2272, epic #2265): blockers, submissions and
//! verification on the Python board and the Rust board, compared after
//! every step by result, refusal text and logical database dump. Ported
//! from `tests/swarm_helpers_test.py`, plus the loosely typed arguments
//! Python accepts (epic P3).
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_both, sql};

/// A running run of five coordinated by `parent`, with `worker` live and
/// holding the claim of task 1 under [`token`]`(3)` (`create` drew ids 1
/// and 2).
fn claimed() -> Vec<Step> {
    vec![
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "res-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
        at(
            3.0,
            "worker",
            "task_create",
            json!(["task", "work", ["tests pass"]]),
        ),
        at(4.0, "worker", "claim", json!([1])),
    ]
}

/// The `n`th id the harness draws, as the counter writes it.
fn token(n: u64) -> String {
    format!("{n:032x}")
}

fn raw(offset: f64, task_id: Value) -> Step {
    at(offset, "parent", "task_raw", json!([task_id]))
}

fn evidence(artifact: &str, revision: &str) -> Value {
    json!([{"artifact": artifact, "revision": revision}])
}

fn scenario(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    let mut steps = claimed();
    steps.extend(more);
    steps
}

/// `test_submission_is_not_completion_and_requires_current_token`: a
/// stale token cannot submit, a submission is not completion, only the
/// coordinator verifies, and verifying twice is idempotent.
#[test]
fn submission_is_not_completion_and_requires_current_token() {
    run_both(&scenario([
        at(
            5.0,
            "worker",
            "submit",
            json!([1, "stale", evidence("report", "abc")]),
        ),
        at(
            6.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("report", "abc")]),
        ),
        raw(7.0, json!(1)),
        at(8.0, "worker", "verify_task", json!([1, token(3), "abc"])),
        at(9.0, "parent", "verify_task", json!([1, token(3), "abc"])),
        at(10.0, "parent", "verify_task", json!([1, token(3), "abc"])),
        raw(11.0, json!(1)),
    ]));
}

/// `test_reviewed_submission_cannot_be_replaced_under_the_same_claim`: an
/// identical re-submit is a safe retry; other evidence, or a blocker, is
/// refused once submitted; the reviewed evidence is what completes.
#[test]
fn reviewed_submission_cannot_be_replaced_under_the_same_claim() {
    run_both(&scenario([
        at(
            5.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("reviewed-A", "R1")]),
        ),
        raw(6.0, json!(1)),
        at(
            7.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("reviewed-A", "R1")]),
        ),
        at(
            8.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("unreviewed-B", "R1")]),
        ),
        at(
            9.0,
            "worker",
            "block",
            json!([1, token(3), "replace via blocked"]),
        ),
        at(10.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        raw(11.0, json!(1)),
    ]));
}

/// `test_released_submission_requires_a_new_review_claim`: after a
/// release the old claim's token no longer verifies; the new claim's
/// submission does.
#[test]
fn released_submission_requires_a_new_review_claim() {
    run_both(&scenario([
        at(
            5.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("A", "R1")]),
        ),
        at(6.0, "worker", "release", json!([1, token(3)])),
        at(7.0, "worker", "claim", json!([1])),
        at(
            8.0,
            "worker",
            "submit",
            json!([1, token(4), evidence("B", "R1")]),
        ),
        at(9.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        at(10.0, "parent", "verify_task", json!([1, token(4), "R1"])),
        raw(11.0, json!(1)),
    ]));
}

/// `test_resolved_blocker_resumes_original_claim_without_releasing_files`,
/// before S10's `reserve`: the file is reserved by an edit instead. The
/// resolved blocker resumes the original claim with its files, only the
/// owner may resume, and submitted evidence cannot be reopened.
#[test]
fn resolved_blocker_resumes_original_claim_without_releasing_files() {
    run_both(&scenario([
        at(
            5.0,
            "worker",
            "block",
            json!([1, token(3), "awaiting approval"]),
        ),
        raw(6.0, json!(1)),
        sql("INSERT INTO files SELECT 'owned.rs', 1, 'worker', token, 'r1' FROM tasks WHERE id=1"),
        at(
            7.0,
            "worker",
            "unblock",
            json!([1, token(3), "approval received"]),
        ),
        raw(8.0, json!(1)),
        at(
            9.0,
            "parent",
            "unblock",
            json!([1, token(3), "not the owner"]),
        ),
        at(
            10.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("tests", "R1")]),
        ),
        at(
            11.0,
            "worker",
            "unblock",
            json!([1, token(3), "cannot reopen submitted evidence"]),
        ),
        at(12.0, "parent", "verify_task", json!([1, token(3), "R1"])),
    ]));
}

/// Blockers: the reason and resolution bounds and their order, a repeated
/// reason, another reason, a claimed task resumed, a submission from a
/// blocked claim, and task ids and tokens bound loosely.
#[test]
fn blockers_are_set_and_resolved_identically() {
    let long = "é".repeat(4096);
    let longer = "é".repeat(4097);
    run_both(&scenario([
        at(5.0, "worker", "block", json!([1, token(3), " \n"])),
        at(5.5, "stranger", "block", json!([1, token(3), 5])),
        at(6.0, "worker", "block", json!([1, token(3), longer])),
        at(6.5, "worker", "unblock", json!([1, token(3), ""])),
        at(7.0, "worker", "unblock", json!([1, token(3), null])),
        at(7.5, "worker", "block", json!([1, "stale", "reason"])),
        at(8.0, "worker", "block", json!(["1", token(3), long])),
        at(8.5, "worker", "block", json!([true, token(3), long])),
        at(9.0, "worker", "block", json!([1.0, token(3), "another"])),
        raw(9.5, json!(1)),
        at(10.0, "worker", "unblock", json!(["1", token(3), "done"])),
        at(10.5, "worker", "unblock", json!([1, token(3), "done"])),
        at(11.0, "worker", "block", json!([1, token(3), "again"])),
        at(
            11.5,
            "worker",
            "submit",
            json!([1, token(3), evidence("a", "R1")]),
        ),
        raw(12.0, json!(1)),
        at(12.5, "worker", "unblock", json!([99, token(3), "r"])),
        at(13.0, "stranger", "unblock", json!([1, token(3), "r"])),
    ]));
}

/// Evidence: every shape `submit` refuses (Python's truthiness), the
/// encoded bound at its edge (ASCII escapes counted), and a re-submit
/// equal by Python's `==` but not by text.
#[test]
fn submitted_evidence_is_checked_identically() {
    let mut steps = claimed();
    let mut offset = 5.0;
    for refused in [
        json!(null),
        json!("report"),
        json!({"artifact": "a", "revision": "r"}),
        json!([]),
        json!(["a"]),
        json!([[]]),
        json!([{}]),
        json!([{"artifact": "a"}]),
        json!([{"revision": "r"}]),
        json!([{"artifact": "", "revision": "r"}]),
        json!([{"artifact": "a", "revision": 0}]),
        json!([{"artifact": "a", "revision": 0.0}]),
        json!([{"artifact": "a", "revision": false}]),
        json!([{"artifact": [], "revision": "r"}]),
        json!([{"artifact": {}, "revision": "r"}]),
        json!([{"artifact": null, "revision": "r"}]),
        json!([{"artifact": "a", "revision": "r"}, {"artifact": "b", "revision": ""}]),
        json!([{"artifact": "a".repeat(8161), "revision": "r"}]),
        json!([{"artifact": "é".repeat(1361), "revision": "r"}]),
    ] {
        steps.push(at(
            offset,
            "worker",
            "submit",
            json!([1, token(3), refused]),
        ));
        offset += 1.0;
    }
    steps.extend([
        at(
            30.0,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "é".repeat(1357), "revision": 1, "n": 1.5, "x": [true]}]]),
        ),
        raw(31.0, json!(1)),
        at(
            32.0,
            "worker",
            "submit",
            json!([1, token(3), [{"x": [1], "n": 1.5, "revision": 1.0, "artifact": "é".repeat(1357)}]]),
        ),
        at(
            33.0,
            "worker",
            "submit",
            json!({"task_id": "1", "token": token(3), "evidence": [{"artifact": "a", "revision": 1}]}),
        ),
        at(34.0, "parent", "verify_task", json!([1, token(3), "1"])),
        at(35.0, "parent", "verify_task", json!([1, token(3), true])),
        raw(36.0, json!(1)),
        at(37.0, "parent", "verify_task", json!([1, token(3), 1.0])),
    ]);
    run_both(&steps);
}

/// Verification: the coordinator gate, a token that is not the claim's,
/// work that is not submitted, a revision any entry does not carry, the
/// claim's files (and no other) deleted, and a task completed by an edit
/// verified with its stored (NULL) token and no evidence.
#[test]
fn verification_is_checked_identically() {
    run_both(&scenario([
        at(4.5, "worker", "task_create", json!(["second", "t", ["ok"]])),
        at(4.6, "parent", "claim", json!([2])),
        at(5.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        at(
            6.0,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "a", "revision": "R1"}, {"artifact": "b", "revision": "R2"}]]),
        ),
        sql(
            "INSERT INTO files SELECT 'a', 1, 'worker', token, 'r1' FROM tasks WHERE id=1;
             INSERT INTO files VALUES('b', 1, 'worker', 'older', 'r2');
             INSERT INTO files SELECT 'c', 2, 'parent', token, 'r3' FROM tasks WHERE id=2;",
        ),
        at(7.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        at(8.0, "parent", "verify_task", json!([1, null, "R1"])),
        at(9.0, "parent", "verify_task", json!([9, token(3), "R1"])),
        at(10.0, "stranger", "verify_task", json!([1, token(3), "R1"])),
        sql("UPDATE tasks SET evidence='[{\"artifact\":\"a\",\"revision\":\"R1\"}]' WHERE id=1"),
        at(
            11.0,
            "parent",
            "verify_task",
            json!({"task_id": true, "token": token(3), "revision": "R1"}),
        ),
        raw(12.0, json!(1)),
        at(13.0, "worker", "release", json!([1, token(3)])),
        at(14.0, "worker", "block", json!([1, token(3), "late"])),
        at(15.0, "worker", "task_create", json!(["third", "t", ["ok"]])),
        sql("UPDATE tasks SET status='completed' WHERE id=3"),
        at(16.0, "parent", "verify_task", json!([3, null, "anything"])),
        at(17.0, "parent", "verify_task", json!([3, "x", "anything"])),
    ]));
}

/// The gate: a dead member neither blocks nor submits, the coordinator
/// cannot verify once the run expired, and the expiry commits before the
/// refusal.
#[test]
fn the_operation_gate_refuses_submissions_identically() {
    run_both(&scenario([
        sql("UPDATE members SET status='dead' WHERE id='worker'"),
        at(5.0, "worker", "block", json!([1, token(3), "r"])),
        at(
            6.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("a", "R1")]),
        ),
        sql("UPDATE members SET status='live' WHERE id='worker'"),
        at(
            7.0,
            "worker",
            "submit",
            json!([1, token(3), evidence("a", "R1")]),
        ),
        at(3_700.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        at(3_701.0, "worker", "unblock", json!([1, token(3), "r"])),
        raw(3_702.0, json!(1)),
    ]));
}

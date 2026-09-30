//! Differential scenarios (#2276, epic #2265): wake notifications
//! (`_notifications` on the sender's side, `_accept_wake` on the
//! receiver's) on the Rust board against the Python board's answers frozen
//! in its golden fixtures (#2283), compared after every step by result,
//! refusal text and logical database dump (which shows where `wake_cursors`
//! sits in `sqlite_master`, so a table created earlier or later than
//! Python's is a difference). Ported from the deleted Python suite `tests/swarm_helpers_test.py`;
//! each test also states what the board answers at the steps the Python
//! test asserts on. `summary()`'s event cursor is the latest event id,
//! counted here: `joined` writes events 1 (`created`), 2 (`reserved`) and 3
//! (`activated`).
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::{inbox, joined, send};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_golden, run_rust, step_text,
};

/// `member`'s `_notifications()`.
pub(crate) fn hints(offset: f64, member: &str) -> Step {
    at(offset, member, "_notifications", json!([]))
}

/// `member`'s `_notifications(True)`.
pub(crate) fn batch(offset: f64, member: &str) -> Step {
    at(offset, member, "_notifications", json!([true]))
}

/// `member`'s `_accept_wake(generation)`.
pub(crate) fn accept(offset: f64, member: &str, generation: Value) -> Step {
    at(offset, member, "_accept_wake", json!([generation]))
}

/// The claim token the `n`th id draw writes (`create` draws 1 and 2).
fn token(n: u64) -> String {
    format!("{n:032x}")
}

fn task_create(offset: f64, member: &str, request: &str, dependencies: Value) -> Step {
    at(
        offset,
        member,
        "task_create",
        json!([request, "implement behavior", ["tests pass"], dependencies]),
    )
}

fn evidence() -> Value {
    json!([{"artifact": "tests.log", "revision": "R1"}])
}

/// `other` admitted and live beside `worker`: events 4 and 5.
fn other() -> [Step; 2] {
    [
        at(2.1, "parent", "_admit", json!(["other", "res-o"])),
        at(
            2.2,
            "parent",
            "_activate",
            json!(["other", "res-o", 12, "o", null]),
        ),
    ]
}

/// `submit_one`: `worker` creates, claims (token 3) and submits task 1,
/// then both sides drain their hints.
fn submit_one(offset: f64) -> [Step; 5] {
    [
        task_create(offset, "worker", "reviewed later", json!([])),
        at(offset + 0.1, "worker", "claim", json!([1])),
        at(
            offset + 0.2,
            "worker",
            "submit",
            json!([1, token(3), evidence()]),
        ),
        hints(offset + 0.3, "parent"),
        hints(offset + 0.4, "worker"),
    ]
}

/// What the Rust board answers at `steps[index]` (Python's answer, once
/// `run_golden` has compared them).
fn answer(steps: &[Step], index: usize) -> Outcome {
    run_rust(&steps[..=index])
}

/// The ids of the members a `_notifications` answer names.
fn woken(outcome: &Outcome) -> Vec<String> {
    let Outcome::Ok(value) = outcome else {
        panic!("not answered: {outcome:?}");
    };
    let members = value.get("members").unwrap_or(value);
    members
        .as_array()
        .unwrap_or_else(|| panic!("a member list: {value}"))
        .iter()
        .map(|member| member["id"].as_str().unwrap().to_owned())
        .collect()
}

fn names(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// Runs `steps` on both boards, then checks the Rust answer at each
/// `(index, expected)`.
fn expect(steps: &[Step], expected: &[(usize, Outcome)]) {
    run_golden(steps);
    for (index, outcome) in expected {
        assert_eq!(&answer(steps, *index), outcome, "step {index}");
    }
}

fn ok(value: Value) -> Outcome {
    Outcome::Ok(value)
}

/// `test_notification_batch_carries_atomic_board_generation`: the batch
/// names the recipient and the generation (the latest event, 4), and a
/// second batch names nobody.
#[test]
fn notification_batch_carries_atomic_board_generation() {
    let steps = joined([
        send(3.0, "parent", "wake-batch", "worker", "action"),
        batch(4.0, "parent"),
        batch(5.0, "parent"),
    ]);
    run_golden(&steps);
    let first = answer(&steps, 4);
    assert_eq!(woken(&first), names(&["worker"]));
    let Outcome::Ok(first) = first else {
        unreachable!()
    };
    assert_eq!(first["generation"], json!(4));
    assert_eq!(
        first["members"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        [
            "id",
            "reservation",
            "status",
            "pid",
            "started",
            "socket",
            "launcher"
        ],
        "every members column, in table order"
    );
    assert_eq!(
        answer(&steps, 5),
        ok(json!({"members": [], "generation": 4}))
    );
}

/// `test_wake_generation_is_consumed_once_and_rechecks_current_work`: a
/// generation wakes once; an acknowledged message wakes nobody; a pause
/// takes priority over a queued hint.
#[test]
fn wake_generation_is_consumed_once_and_rechecks_current_work() {
    let steps = joined([
        send(3.0, "worker", "wake", "parent", "review this"),
        accept(4.0, "parent", json!(4)),
        accept(5.0, "parent", json!(4)),
        at(6.0, "parent", "ack", json!([1])),
        accept(7.0, "parent", json!(5)),
        send(8.0, "worker", "wake-2", "parent", "new work"),
        at(
            9.0,
            "parent",
            "pause",
            json!(["pause takes priority over queued hint"]),
        ),
        accept(10.0, "parent", json!(6)),
    ]);
    expect(
        &steps,
        &[
            (4, ok(json!(true))),
            (5, ok(json!(false))),
            (7, ok(json!(false))),
            (10, ok(json!(false))),
        ],
    );
}

/// `test_paused_run_retains_the_wake_frontier_until_resume`: a paused run
/// answers `False` without consuming the generation, which wakes after
/// the supervisor's resume; a generation ahead of the board is refused.
#[test]
fn paused_run_retains_the_wake_frontier_until_resume() {
    let steps = joined([
        send(3.0, "worker", "wake-paused", "parent", "work"),
        at(4.0, "parent", "pause", json!(["hold"])),
        accept(5.0, "parent", json!(4)),
        at(6.0, "parent", "_resume_external", json!([])),
        accept(7.0, "parent", json!(4)),
        accept(8.0, "parent", json!(7)),
    ]);
    expect(
        &steps,
        &[
            (5, ok(json!(false))),
            (7, ok(json!(true))),
            (
                8,
                Outcome::Refused("wake generation is ahead of the board".to_owned()),
            ),
        ],
    );
}

/// `test_unchanged_blocker_does_not_repeat_idle_wake_cycles`.
#[test]
fn unchanged_blocker_does_not_repeat_idle_wake_cycles() {
    let block = |offset, reason: &str| at(offset, "worker", "block", json!([1, token(3), reason]));
    let mut steps = joined([
        task_create(3.0, "worker", "task", json!([])),
        at(4.0, "worker", "claim", json!([1])),
        block(5.0, "awaiting approval"),
        hints(6.0, "worker"),
    ]);
    for round in 0..3 {
        let offset = 7.0 + f64::from(round);
        steps.push(inbox(offset, "parent", json!(false)));
        steps.push(block(offset + 0.1, "awaiting approval"));
        steps.push(hints(offset + 0.2, "worker"));
    }
    steps.push(block(11.0, "new blocker"));
    steps.push(hints(12.0, "worker"));
    run_golden(&steps);
    assert_eq!(woken(&answer(&steps, 6)), names(&["parent"]));
    assert_eq!(woken(&answer(&steps, 9)), names(&[]));
    assert_eq!(woken(&answer(&steps, steps.len() - 1)), names(&["parent"]));
}

/// `test_identical_evidence_does_not_repeat_review_wake`.
#[test]
fn identical_evidence_does_not_repeat_review_wake() {
    let evidence = |offset| {
        at(
            offset,
            "worker",
            "evidence",
            json!(["t", "tests.log", "R1", "command", true]),
        )
    };
    let steps = joined([
        evidence(3.0),
        hints(4.0, "worker"),
        evidence(5.0),
        hints(6.0, "worker"),
    ]);
    run_golden(&steps);
    assert_eq!(woken(&answer(&steps, 4)), names(&["parent"]));
    assert_eq!(woken(&answer(&steps, 6)), names(&[]));
}

/// `test_dependency_work_only_wakes_when_claimable`.
#[test]
fn dependency_work_only_wakes_when_claimable() {
    let steps = joined([
        task_create(3.0, "worker", "dependency", json!([])),
        at(4.0, "worker", "claim", json!([1])),
        hints(5.0, "worker"),
        task_create(6.0, "worker", "dependent", json!([1])),
        hints(7.0, "worker"),
        at(8.0, "worker", "submit", json!([1, token(3), evidence()])),
        at(9.0, "parent", "verify_task", json!([1, token(3), "R1"])),
        hints(10.0, "worker"),
        hints(11.0, "parent"),
    ]);
    run_golden(&steps);
    assert_eq!(woken(&answer(&steps, 7)), names(&[]));
    assert_eq!(
        woken(&answer(&steps, 10)),
        names(&[]),
        "already verified submission is stale"
    );
    assert_eq!(woken(&answer(&steps, 11)), names(&["worker"]));
}

/// `test_consumed_work_does_not_emit_stale_wake_hints`.
#[test]
fn consumed_work_does_not_emit_stale_wake_hints() {
    let steps = joined([
        hints(3.0, "parent"),
        hints(4.0, "worker"),
        send(5.0, "worker", "already-read", "parent", "Please review"),
        at(6.0, "parent", "ack", json!([1])),
        hints(7.0, "worker"),
    ]);
    expect(&steps, &[(7, ok(json!([])))]);
}

/// `test_a_member_that_submitted_is_parked_while_another_worker_is_free`
/// (#2127): the free worker is woken and accepts the generation (9); the
/// parked one does not; a message still wakes it.
#[test]
fn a_member_that_submitted_is_parked_while_another_worker_is_free() {
    let mut steps = joined(other());
    steps.extend(submit_one(3.0));
    steps.extend([
        at(
            4.0,
            "parent",
            "task_create",
            json!(["someone else", "implement", ["tests pass"]]),
        ),
        batch(5.0, "parent"),
        accept(6.0, "worker", json!(9)),
        accept(7.0, "other", json!(9)),
        send(
            8.0,
            "parent",
            "please-rework",
            "worker",
            "Rework the evidence",
        ),
        hints(9.0, "parent"),
    ]);
    run_golden(&steps);
    let batched = answer(&steps, 11);
    assert_eq!(woken(&batched), names(&["other"]));
    assert!(matches!(&batched, Outcome::Ok(value) if value["generation"] == json!(9)));
    assert_eq!(answer(&steps, 12), ok(json!(false)));
    assert_eq!(answer(&steps, 13), ok(json!(true)));
    assert_eq!(woken(&answer(&steps, 15)), names(&["worker"]));
}

/// `test_a_parked_member_takes_new_work_when_no_worker_is_free`.
#[test]
fn a_parked_member_takes_new_work_when_no_worker_is_free() {
    let mut steps = joined(submit_one(3.0));
    steps.extend([
        at(
            4.0,
            "parent",
            "task_create",
            json!(["only you", "implement", ["tests pass"]]),
        ),
        hints(5.0, "parent"),
    ]);
    run_golden(&steps);
    assert_eq!(woken(&answer(&steps, 9)), names(&["worker"]));
}

/// `test_a_release_wakes_the_parked_member_and_its_own_check_agrees`: the
/// releaser's event hands the work to the parked worker, which judges it
/// from the releaser's view and accepts (generation 11).
#[test]
fn a_release_wakes_the_parked_member_and_its_own_check_agrees() {
    let mut steps = joined(other());
    steps.extend(submit_one(3.0));
    steps.extend([
        task_create(4.0, "worker", "conflicting files", json!([])),
        at(5.0, "other", "claim", json!([2])),
        hints(6.0, "parent"),
        hints(7.0, "other"),
        at(8.0, "other", "release", json!([2, token(4)])),
        batch(9.0, "other"),
        accept(10.0, "worker", json!(11)),
    ]);
    run_golden(&steps);
    let batched = answer(&steps, 15);
    assert_eq!(woken(&batched), names(&["parent", "worker"]));
    assert!(matches!(&batched, Outcome::Ok(value) if value["generation"] == json!(11)));
    assert_eq!(answer(&steps, 16), ok(json!(true)));
}

/// `test_claimed_work_does_not_wake_idle_peers`.
#[test]
fn claimed_work_does_not_wake_idle_peers() {
    let steps = joined([
        hints(3.0, "parent"),
        hints(4.0, "worker"),
        task_create(5.0, "worker", "task", json!([])),
        at(6.0, "worker", "claim", json!([1])),
        hints(7.0, "worker"),
    ]);
    expect(&steps, &[(7, ok(json!([])))]);
}

/// `test_wake_hints_are_targeted_deduplicated_and_terminal_safe`.
#[test]
fn wake_hints_are_targeted_deduplicated_and_terminal_safe() {
    let steps = joined([
        hints(3.0, "parent"),
        hints(4.0, "worker"),
        send(5.0, "worker", "question", "parent", "Review my result"),
        hints(6.0, "worker"),
        hints(7.0, "worker"),
        at(8.0, "parent", "ack", json!([1])),
        hints(9.0, "parent"),
        send(10.0, "worker", "late-question", "parent", "Late result"),
        at(
            11.0,
            "parent",
            "stop",
            json!(["blocked", "report partial progress"]),
        ),
        hints(12.0, "worker"),
    ]);
    run_golden(&steps);
    assert_eq!(woken(&answer(&steps, 6)), names(&["parent"]));
    assert_eq!(answer(&steps, 7), ok(json!([])));
    assert_eq!(
        answer(&steps, 9),
        ok(json!([])),
        "acknowledgment must not wake the pool"
    );
    assert_eq!(
        answer(&steps, 12),
        ok(json!([])),
        "an ended run suppresses queued hints"
    );
}

/// `test_an_observers_read_does_not_broadcast_another_members_changes`
/// (the observer reads its inbox where Python reads `summary()`).
#[test]
fn an_observers_read_does_not_broadcast_another_members_changes() {
    let steps = joined([
        hints(3.0, "parent"),
        at(
            4.0,
            "worker",
            "task_create",
            json!(["new", "work", ["pass"]]),
        ),
        inbox(5.0, "parent", json!(false)),
        hints(6.0, "parent"),
        hints(7.0, "worker"),
    ]);
    run_golden(&steps);
    assert_eq!(answer(&steps, 6), ok(json!([])));
    assert_eq!(woken(&answer(&steps, 7)), names(&["parent"]));
}

/// `test_a_withdrawn_message_leaves_the_inbox_and_wakes_nobody` (its wake
/// half).
#[test]
fn a_withdrawn_message_wakes_nobody() {
    let steps = joined([
        hints(3.0, "worker"),
        at(
            4.0,
            "worker",
            "send",
            json!(["w1", "parent", "never mind", "abc1"]),
        ),
        at(5.0, "worker", "withdraw", json!([1])),
        hints(6.0, "worker"),
    ]);
    expect(&steps, &[(6, ok(json!([])))]);
}

/// `_accept_wake` takes a nonnegative `int` (`-0` is 0; a boolean, a
/// float, text or null is refused before the gate), by position or by
/// name; an integer beyond the board is ahead of it; a stranger is
/// refused by the gate; a stopped run wakes nobody.
#[test]
fn accept_wake_checks_its_generation_as_python_does() {
    let mut steps = joined([send(3.0, "worker", "a", "parent", "one")]);
    for generation in [
        json!(true),
        json!(1.0),
        json!(-1),
        json!("1"),
        Value::Null,
        json!([1]),
    ] {
        steps.push(accept(4.0, "parent", generation));
    }
    steps.extend([
        step_text("parent", "_accept_wake", "[-0]", 5.0),
        accept(6.0, "parent", json!(u64::MAX)),
        accept(7.0, "parent", json!(9_223_372_036_854_775_807_i64)),
        at(8.0, "parent", "_accept_wake", json!({"generation": 4})),
        accept(9.0, "stranger", json!(4)),
        at(10.0, "parent", "stop", json!(["blocked", "done"])),
        accept(11.0, "worker", json!(4)),
    ]);
    run_golden(&steps);
    assert_eq!(
        answer(&steps, 4),
        Outcome::Refused("wake generation must be a nonnegative integer".to_owned())
    );
}

/// `_notifications` answers the batch by Python's truth of
/// `with_generation`, by position or by name.
#[test]
fn notifications_take_with_generation_by_its_truth() {
    let mut steps = joined([]);
    for (index, flag) in [
        json!(1),
        json!(0),
        json!(0.0),
        json!(""),
        json!("x"),
        json!([]),
        json!([0]),
        json!({}),
        Value::Null,
    ]
    .into_iter()
    .enumerate()
    {
        steps.push(send(
            3.0 + index as f64,
            "parent",
            &format!("m{index}"),
            "worker",
            "hi",
        ));
        steps.push(at(
            3.5 + index as f64,
            "parent",
            "_notifications",
            json!([flag]),
        ));
    }
    steps.push(at(
        20.0,
        "parent",
        "_notifications",
        json!({"with_generation": true}),
    ));
    steps.push(at(21.0, "stranger", "_notifications", json!([])));
    run_golden(&steps);
}

/// Wake ops are reads (`operation(active=False, read_only=True)`), so a
/// paused run answers them with Python's exact values and texts: no hint,
/// and no wake, while the frontier is kept (#2316's paused-run table).
#[test]
fn every_wake_op_on_a_paused_run() {
    let setup = joined([send(3.0, "worker", "a", "parent", "one")]);
    let paused = crate::swarm_board_diff_runs::swarm_board_diff::scenario::sql(
        "UPDATE run SET status='paused'",
    );
    for (member, method, args, running, held) in [
        (
            "worker",
            "_notifications",
            json!([true]),
            json!({"members": [{"id": "parent"}], "generation": 4}),
            json!({"members": [], "generation": 4}),
        ),
        (
            "parent",
            "_accept_wake",
            json!([4]),
            json!(true),
            json!(false),
        ),
    ] {
        let op = at(9.0, member, method, args);
        let granted = [setup.clone(), vec![op.clone()]].concat();
        let answered = run_rust(&granted);
        match (&answered, method) {
            (Outcome::Ok(value), "_notifications") => {
                assert_eq!(woken(&answered), names(&["parent"]));
                assert_eq!(value["generation"], running["generation"]);
            }
            _ => assert_eq!(answered, ok(running), "{method} on the running run"),
        }
        let steps = [setup.clone(), vec![paused.clone(), op.clone(), op]].concat();
        run_golden(&steps);
        assert_eq!(run_rust(&steps), ok(held), "{method} on the paused run");
    }
}

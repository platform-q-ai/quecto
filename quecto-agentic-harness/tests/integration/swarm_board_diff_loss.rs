//! Differential scenarios (#2277, epic #2265): loss and death recording
//! (`_quarantine`, `_confirmed_dead` and `_lose_coordinator`, #1924,
//! #1961) on the Python board and the Rust board, compared after every
//! step by result, refusal text and logical database dump (the events
//! each op writes, their details and times included). Ported from
//! `tests/swarm_helpers_test.py`, plus the loosely typed arguments Python
//! accepts (epic P3) and the paused-run table.
//!
//! The Python tests read the run through `summary()` and `events()`, which
//! the read-model half of S12 ports; here `_snapshot` (status, outcome,
//! deadline, control generation, member rows), `_control_status` (the
//! outcome's reason), `task_raw` and `file_owners` state the same facts,
//! and the dump compares every event. `joined` writes events 1
//! (`created`), 2 (`reserved`) and 3 (`activated`); the worker's first
//! claim is token 3.
use serde_json::{Value, json};

use crate::swarm_board_diff_files::token;
use crate::swarm_board_diff_membership::{HOUR, at};
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_both, run_rust, step};

/// `Workbench.LOSS_GRACE`, in seconds.
pub(crate) const GRACE: f64 = 10.0;

pub(crate) fn quarantine(offset: f64, observer: &str, member: Value) -> Step {
    at(offset, observer, "_quarantine", json!([member]))
}

/// `_confirmed_dead(member)`, orderly by default.
pub(crate) fn dead(offset: f64, observer: &str, member: &str) -> Step {
    at(offset, observer, "_confirmed_dead", json!([member]))
}

pub(crate) fn abrupt(offset: f64, observer: &str, member: &str) -> Step {
    at(
        offset,
        observer,
        "_confirmed_dead",
        json!([member, "abrupt"]),
    )
}

/// The test's `lose`: a first observation at `offset`, then one a grace
/// later.
pub(crate) fn lose(offset: f64, observer: &str, member: &str) -> [Step; 2] {
    [
        quarantine(offset, observer, json!(member)),
        quarantine(offset + GRACE, observer, json!(member)),
    ]
}

pub(crate) fn snapshot(offset: f64, member: &str) -> Step {
    at(offset, member, "_snapshot", json!([]))
}

pub(crate) fn raw(offset: f64, task: i64) -> Step {
    at(offset, "parent", "task_raw", json!([task]))
}

pub(crate) fn owners(offset: f64) -> Step {
    at(offset, "parent", "file_owners", json!([]))
}

/// `member` creates a task as `request` (task 1 first) at `offset`.
pub(crate) fn task(offset: f64, member: &str, request: &str) -> Step {
    at(
        offset,
        member,
        "task_create",
        json!([request, "implement behavior", ["tests pass"], []]),
    )
}

/// `member` admitted and activated by `launcher` at `offset`.
pub(crate) fn launched(offset: f64, launcher: &str, member: &str) -> [Step; 2] {
    let reservation = format!("reservation-{member}");
    [
        at(offset, launcher, "_admit", json!([member, reservation])),
        at(
            offset + 0.1,
            launcher,
            "_activate",
            json!([member, reservation, 12_346, "start", null]),
        ),
    ]
}

/// The Rust board's answer at `steps[index]` (Python's, once `run_both`
/// has compared them).
pub(crate) fn answer(steps: &[Step], index: usize) -> Value {
    match run_rust(&steps[..=index]) {
        Outcome::Ok(value) => value,
        other => panic!("step {index} was not answered: {other:?}"),
    }
}

pub(crate) fn refusal(steps: &[Step], index: usize) -> String {
    match run_rust(&steps[..=index]) {
        Outcome::Refused(text) => text,
        other => panic!("step {index} was not refused: {other:?}"),
    }
}

/// The run's `(status, outcome)` in a `_snapshot` answer.
pub(crate) fn state(snapshot: &Value) -> (Value, Value) {
    (snapshot["status"].clone(), snapshot["outcome"].clone())
}

pub(crate) fn held(outcome: &str) -> (Value, Value) {
    (json!("paused"), json!(outcome))
}

pub(crate) fn running() -> (Value, Value) {
    (json!("running"), Value::Null)
}

const NO_NEW_WORK: &str = "run is paused (failed: ";

/// `test_coordinator_death_leaves_readable_failed_progress`.
#[test]
fn coordinator_death_leaves_readable_failed_progress() {
    let steps = joined([
        task(3.0, "worker", "task"),
        dead(4.0, "worker", "parent"),
        snapshot(5.0, "parent"),
        task(6.0, "worker", "after-parent-death"),
        raw(7.0, 1),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 5)), held("failed"));
    assert!(refusal(&steps, 6).starts_with(NO_NEW_WORK));
    assert_eq!(answer(&steps, 7)["id"], json!(1), "progress stays readable");
}

/// `test_status_and_loss_receipts_name_the_run`: `_status` and
/// `_lose_coordinator` carry the run's id, the setup placeholder's too.
#[test]
fn status_and_loss_receipts_name_the_run() {
    let steps = joined([
        at(3.0, "supervisor", "_status", json!([])),
        at(4.0, "parent", "_lose_coordinator", json!([])),
    ]);
    run_both(&steps);
    let run_id = answer(&steps, 3)["id"].clone();
    assert!(run_id.as_str().is_some_and(|id| !id.is_empty()));
    let receipt = answer(&steps, 4);
    assert_eq!(receipt["id"], run_id);
    assert_eq!(receipt["lost"], json!(true));
    let fresh = [
        step(
            "boot",
            "bootstrap_run",
            json!([1, "start-b", "/tmp/b.sock"]),
            NOW,
        ),
        step("supervisor", "_status", json!([]), NOW + 1.0),
    ];
    run_both(&fresh);
    assert!(
        answer(&fresh, 1)["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

/// `test_a_lost_coordinator_ends_the_run_as_a_failed_pause_even_while_paused`:
/// the pause keeps its record (the control generation does not move) and
/// takes `failed` with the coordinator's death as its reason.
#[test]
fn a_lost_coordinator_ends_the_run_as_a_failed_pause_even_while_paused() {
    let steps = joined([
        at(3.0, "parent", "pause", json!(["hold"])),
        snapshot(3.5, "worker"),
        dead(4.0, "worker", "parent"),
        snapshot(5.0, "worker"),
        at(5.5, "worker", "_control_status", json!([])),
        task(6.0, "worker", "after-parent-death"),
    ]);
    run_both(&steps);
    let (before, after) = (answer(&steps, 4), answer(&steps, 6));
    assert_eq!(state(&after), held("failed"));
    assert_eq!(after["control_generation"], before["control_generation"]);
    assert_eq!(
        answer(&steps, 7)["reason"],
        json!("coordinator death confirmed")
    );
    assert!(refusal(&steps, 8).starts_with(NO_NEW_WORK));
}

/// `test_a_loss_during_a_pause_keeps_the_frozen_budget`: a worker lost a
/// hundred seconds into a pause leaves the pause's start, so the resume
/// adds the whole paused interval back to the deadline.
#[test]
fn a_loss_during_a_pause_keeps_the_frozen_budget() {
    let mut steps = joined([at(3.0, "parent", "pause", json!(["hold"]))]);
    steps.extend(lose(103.0, "parent", "worker"));
    steps.extend([
        snapshot(120.0, "parent"),
        at(503.0, "parent", "_resume_external", json!([])),
        snapshot(504.0, "parent"),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 6)), held("failed"));
    assert_eq!(answer(&steps, 8)["deadline"], json!(NOW + HOUR + 500.0));
}

/// `test_a_worker_loss_keeps_the_verdict_the_coordinator_already_proposed`.
#[test]
fn a_worker_loss_keeps_the_verdict_the_coordinator_already_proposed() {
    let mut steps = joined([
        at(
            3.0,
            "parent",
            "evidence",
            json!(["t", "ci.log", "R1", "command", true]),
        ),
        at(4.0, "parent", "complete", json!(["R1"])),
    ]);
    steps.extend(lose(5.0, "parent", "worker"));
    steps.extend([
        snapshot(20.0, "parent"),
        at(21.0, "parent", "_close", json!([])),
        snapshot(22.0, "parent"),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 7)), held("succeeded"));
    assert_eq!(answer(&steps, 9)["status"], json!("succeeded"));
}

/// `test_revoke_after_confirmed_death_records_the_audit_without_a_message`.
#[test]
fn revoke_after_confirmed_death_records_the_audit_without_a_message() {
    let steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        dead(5.0, "parent", "worker"),
        raw(6.0, 1),
        at(7.0, "parent", "revoke", json!([1, "reassign after death"])),
    ]);
    run_both(&steps);
    assert_eq!(answer(&steps, 6)["status"], json!("blocked"));
    assert_eq!(answer(&steps, 7)["status"], json!("ready"));
}

/// `test_a_lost_member_is_recorded_once_and_never_pauses_a_resumed_run_again`.
#[test]
fn a_lost_member_is_recorded_once_and_never_pauses_a_resumed_run_again() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
    ]);
    steps.extend(lose(5.0, "parent", "worker"));
    steps.extend([
        snapshot(16.0, "parent"),
        at(17.0, "parent", "_resume_external", json!([])),
        snapshot(18.0, "parent"),
    ]);
    // The stale observation replays on every later reconcile.
    steps.extend(lose(118.0, "parent", "worker"));
    steps.extend([
        snapshot(129.0, "parent"),
        dead(130.0, "parent", "worker"),
        snapshot(131.0, "parent"),
        raw(132.0, 1),
        quarantine(133.0, "parent", json!("worker")),
        snapshot(134.0, "parent"),
        at(135.0, "parent", "recover", json!([1])),
        at(136.0, "parent", "claim", json!([1])),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 7)), held("failed"));
    for index in [9, 12, 14, 17] {
        assert_eq!(state(&answer(&steps, index)), running(), "step {index}");
    }
    assert_eq!(answer(&steps, 15)["status"], json!("blocked"));
    assert_eq!(answer(&steps, 19)["owner"], json!("parent"));
}

/// `test_a_confirmed_member_death_keeps_the_run_running_and_blocks_its_work`.
#[test]
fn a_confirmed_member_death_keeps_the_run_running_and_blocks_its_work() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        at(5.0, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        dead(6.0, "parent", "worker"),
        snapshot(7.0, "parent"),
        owners(8.0),
        raw(9.0, 1),
        at(10.0, "parent", "_control_status", json!([])),
        at(11.0, "parent", "_notifications", json!([true])),
        at(12.0, "parent", "recover", json!([1])),
    ]);
    steps.extend(launched(13.0, "parent", "other"));
    steps.push(at(14.0, "other", "claim", json!([1])));
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 7)), running());
    assert_eq!(answer(&steps, 8), json!([]));
    let blocked = answer(&steps, 9);
    assert_eq!(
        (&blocked["status"], &blocked["owner"], &blocked["blocker"]),
        (
            &json!("blocked"),
            &json!("worker"),
            &json!("worker death confirmed; coordinator recovery required")
        )
    );
    assert_eq!(answer(&steps, 10)["resume_blockers"], json!([]));
    assert_eq!(answer(&steps, 11)["members"], json!([]));
    assert_eq!(answer(&steps, 15)["owner"], json!("other"));
}

/// `test_only_the_launcher_records_a_member_loss_and_only_after_the_grace`:
/// another member observes and records nothing while the launcher lives;
/// the launcher's first observation starts the grace, once; a death its
/// reaper confirms inside the grace never pauses the run.
#[test]
fn only_the_launcher_records_a_member_loss_and_only_after_the_grace() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
    ]);
    steps.extend(launched(5.0, "parent", "other"));
    let base = 10.0;
    for offset in [0.0, 5.0, 60.0, 200.0] {
        steps.push(quarantine(base + offset, "other", json!("worker")));
    }
    steps.extend([
        snapshot(base + 201.0, "parent"),
        quarantine(base, "parent", json!("worker")),
        snapshot(base + 0.5, "parent"),
        quarantine(base + GRACE - 1.0, "parent", json!("worker")),
        snapshot(base + GRACE - 0.5, "parent"),
        dead(base + GRACE - 0.2, "parent", "worker"),
        quarantine(base + GRACE, "parent", json!("worker")),
        snapshot(base + GRACE + 0.5, "parent"),
        raw(base + GRACE + 0.6, 1),
        at(base + GRACE + 0.7, "parent", "recover", json!([1])),
    ]);
    run_both(&steps);
    for index in [11, 13, 15, 18] {
        assert_eq!(state(&answer(&steps, index)), running(), "step {index}");
    }
    assert_eq!(answer(&steps, 19)["status"], json!("blocked"));
}

/// `test_the_launcher_records_a_loss_its_reaper_never_confirmed_after_the_grace`.
#[test]
fn the_launcher_records_a_loss_its_reaper_never_confirmed_after_the_grace() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
    ]);
    steps.extend(lose(5.0, "parent", "worker"));
    steps.extend([snapshot(16.0, "parent"), raw(17.0, 1)]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 7)), held("failed"));
    assert_eq!(
        answer(&steps, 8)["owner"],
        json!("worker"),
        "ownership retained"
    );
}

/// `test_any_member_records_a_loss_once_the_launcher_itself_is_dead`: the
/// worker launched `nested` and then died.
#[test]
fn any_member_records_a_loss_once_the_launcher_itself_is_dead() {
    let mut steps = joined(launched(3.0, "worker", "nested"));
    let base = 10.0;
    steps.extend([
        quarantine(base, "parent", json!("nested")),
        snapshot(base, "parent"),
        dead(base, "parent", "worker"),
        snapshot(base, "parent"),
        quarantine(base, "parent", json!("nested")),
        snapshot(base, "parent"),
        quarantine(base + GRACE, "parent", json!("nested")),
        snapshot(base + GRACE, "parent"),
    ]);
    run_both(&steps);
    for index in [6, 8, 10] {
        assert_eq!(state(&answer(&steps, index)), running(), "step {index}");
    }
    assert_eq!(state(&answer(&steps, 12)), held("failed"));
}

/// `test_a_launcher_less_member_loss_is_recorded_at_once`: the created
/// coordinator has no launcher.
#[test]
fn a_launcher_less_member_loss_is_recorded_at_once() {
    let steps = joined([
        quarantine(3.0, "worker", json!("parent")),
        snapshot(4.0, "worker"),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 4)), held("failed"));
}

/// `test_an_orderly_exit_releases_reservations_and_an_abrupt_one_retains_them`.
#[test]
fn an_orderly_exit_releases_reservations_and_an_abrupt_one_retains_them() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        at(5.0, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        at(6.0, "parent", "_confirmed_dead", json!(["worker", "maybe"])),
        abrupt(7.0, "parent", "worker"),
        raw(8.0, 1),
        owners(9.0),
        at(10.0, "parent", "recover", json!([1])),
        owners(11.0),
    ]);
    steps.extend(launched(12.0, "parent", "other"));
    steps.extend([
        task(13.0, "other", "o"),
        at(14.0, "other", "claim", json!([2])),
        at(15.0, "other", "reserve", json!([2, token(5), ["src/a.rs"]])),
        at(16.0, "parent", "recover", json!([1, true])),
        owners(17.0),
    ]);
    steps.extend(launched(18.0, "parent", "tidy"));
    steps.extend([
        task(19.0, "tidy", "t2"),
        at(20.0, "tidy", "claim", json!([3])),
        at(21.0, "tidy", "reserve", json!([3, token(6), ["src/b.rs"]])),
        dead(22.0, "parent", "tidy"),
        owners(23.0),
        raw(24.0, 3),
        at(25.0, "parent", "recover", json!([3])),
    ]);
    run_both(&steps);
    assert_eq!(refusal(&steps, 6), "exit kind must be orderly or abrupt");
    let blocked = answer(&steps, 8);
    assert_eq!(blocked["status"], json!("blocked"));
    assert!(
        blocked["blocker"]
            .as_str()
            .unwrap()
            .contains("reservations retained")
    );
    assert_eq!(answer(&steps, 9)[0]["path"], json!("src/a.rs"));
    assert!(refusal(&steps, 10).contains("release_files=True"));
    assert_eq!(answer(&steps, 11).as_array().map(Vec::len), Some(1));
    assert!(refusal(&steps, 16).contains("already reserved"));
    assert_eq!(answer(&steps, 18), json!([]));
    assert_eq!(answer(&steps, 25), json!([]));
    assert_eq!(
        answer(&steps, 26)["blocker"],
        json!("worker death confirmed; coordinator recovery required")
    );
}

/// `test_revoke_frees_reservations_retained_after_an_abrupt_exit_with_the_reason`.
#[test]
fn revoke_frees_reservations_retained_after_an_abrupt_exit_with_the_reason() {
    let steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        at(5.0, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        abrupt(6.0, "parent", "worker"),
        at(
            7.0,
            "parent",
            "revoke",
            json!([1, "cargo test orphan confirmed gone"]),
        ),
        owners(8.0),
        raw(9.0, 1),
    ]);
    run_both(&steps);
    assert_eq!(answer(&steps, 8), json!([]));
    assert_eq!(answer(&steps, 9)["status"], json!("ready"));
}

/// `test_the_loss_grace_runs_from_the_first_authorised_observation_by_anyone`:
/// another observer's first observation, a grace after the coordinator's,
/// records the loss at once.
#[test]
fn the_loss_grace_runs_from_the_first_authorised_observation_by_anyone() {
    let mut steps = joined(launched(3.0, "worker", "nested"));
    steps.push(dead(4.0, "parent", "worker"));
    steps.extend(launched(5.0, "parent", "other"));
    let base = 10.0;
    steps.extend([
        quarantine(base, "parent", json!("nested")),
        snapshot(base, "parent"),
        quarantine(base + GRACE, "other", json!("nested")),
        snapshot(base + GRACE, "parent"),
    ]);
    run_both(&steps);
    assert_eq!(state(&answer(&steps, 9)), running());
    assert_eq!(state(&answer(&steps, 11)), held("failed"));
}

/// A confirmed death blocks every active task of the member (claimed,
/// blocked or submitted) and no other; a task the member completed keeps
/// its status.
#[test]
fn a_confirmed_death_blocks_every_active_task_and_no_other() {
    let steps = joined([
        task(3.0, "worker", "claimed"),
        task(3.1, "worker", "blocked"),
        task(3.2, "worker", "submitted"),
        task(3.3, "worker", "completed"),
        task(3.4, "worker", "ready"),
        at(4.0, "worker", "claim", json!([1])),
        at(4.1, "worker", "claim", json!([2])),
        at(4.2, "worker", "block", json!([2, token(4), "waiting"])),
        at(4.3, "worker", "claim", json!([3])),
        at(
            4.4,
            "worker",
            "submit",
            json!([3, token(5), [{"artifact": "a", "revision": "r"}]]),
        ),
        at(4.5, "worker", "claim", json!([4])),
        at(
            4.6,
            "worker",
            "submit",
            json!([4, token(6), [{"artifact": "a", "revision": "r"}]]),
        ),
        at(4.7, "parent", "verify_task", json!([4, token(6), "r"])),
        abrupt(5.0, "parent", "worker"),
        dead(6.0, "parent", "worker"),
    ]);
    run_both(&steps);
    for (task, status) in [
        (1, "blocked"),
        (2, "blocked"),
        (3, "blocked"),
        (4, "completed"),
        (5, "ready"),
    ] {
        let mut probe = steps.clone();
        probe.push(raw(7.0, task));
        assert_eq!(
            answer(&probe, probe.len() - 1)["status"],
            json!(status),
            "task {task}"
        );
    }
}

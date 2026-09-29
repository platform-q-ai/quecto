//! Differential scenarios (#2273, epic #2265): run control, the control
//! receipt and the usage report on the Python board and the Rust board,
//! compared after every step by result, refusal text and logical database
//! dump. Ported from `tests/swarm_helpers_test.py`, plus the loosely typed
//! arguments Python accepts (epic P3). `complete`, `revalidate_task` and
//! `amend` are a later slice: a proposed outcome here is `stop`'s.
use serde_json::json;

use crate::swarm_board_diff_membership::{HOUR, at, create, snapshot};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_both, sql, step, step_text,
};

/// A running run of five coordinated by `parent`, with `worker` live.
fn with_worker(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    let mut steps = vec![
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "res-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
    ];
    steps.extend(more);
    steps
}

fn control(offset: f64, member: &str, method: &str) -> Step {
    at(offset, member, method, json!([]))
}

fn task(offset: f64, title: &str) -> Step {
    at(
        offset,
        "worker",
        "task_create",
        json!([title, title, ["ok"]]),
    )
}

/// `test_pause_is_durable_freezes_budget_and_rejects_mutation`: a pause
/// outlives the deadline without expiring, refuses new work and member
/// resumes, and the supervisor's resume moves the deadline on by the
/// paused interval.
#[test]
fn pause_is_durable_freezes_budget_and_rejects_mutation() {
    let late = HOUR + 500.0;
    run_both(&with_worker([
        at(
            5.0,
            "parent",
            "pause",
            json!(["operator requested preservation"]),
        ),
        snapshot(late),
        task(late, "cannot start during pause"),
        control(late, "worker", "resume"),
        control(late, "parent", "resume"),
        control(late, "worker", "_control_status"),
        control(late, "parent", "_resume_external"),
        snapshot(late + 1.0),
        task(late + 2.0, "new change"),
        at(late + 3.0, "parent", "pause", json!({"reason": "again"})),
        at(late + 4.0, "parent", "pause", json!(["and again"])),
        control(late + 5.0, "parent", "_control_status"),
    ]));
}

/// `test_coordinator_stop_is_a_resumable_pause_only_the_supervisor_lifts`:
/// a stop pauses holding the outcome, kills nobody and keeps claims; only
/// the supervisor lifts it; the same stop again is a no-op and another
/// verdict is refused.
#[test]
fn coordinator_stop_is_a_resumable_pause_only_the_supervisor_lifts() {
    run_both(&with_worker([
        task(3.0, "work"),
        at(4.0, "worker", "claim", json!([1])),
        at(
            5.0,
            "parent",
            "stop",
            json!(["blocked", "needs a decision from the master"]),
        ),
        snapshot(6.0),
        at(7.0, "parent", "task_raw", json!([1])),
        control(8.0, "parent", "_control_status"),
        task(9.0, "nothing new while ended"),
        control(10.0, "parent", "resume"),
        control(11.0, "worker", "resume"),
        at(
            12.0,
            "parent",
            "stop",
            json!({"reason": "needs a decision from the master", "status": "blocked"}),
        ),
        at(
            13.0,
            "parent",
            "stop",
            json!(["failed", "a different verdict while ended"]),
        ),
        control(14.0, "parent", "_resume_external"),
        snapshot(15.0),
        at(16.0, "parent", "task_raw", json!([1])),
    ]));
}

/// `test_completion_holds_success_until_the_supervisor_closes_it`, with
/// `stop`'s outcome standing in for `complete`'s: a running run and a
/// plain pause hold nothing to close; a held outcome is closed only by the
/// supervisor, after which the run is terminal.
#[test]
fn a_proposed_outcome_is_held_until_the_supervisor_closes_it() {
    run_both(&with_worker([
        control(3.0, "parent", "_close"),
        at(
            4.0,
            "parent",
            "pause",
            json!(["a plain pause holds nothing"]),
        ),
        control(5.0, "parent", "_close"),
        control(6.0, "parent", "_resume_external"),
        at(7.0, "parent", "stop", json!(["budget-exhausted", "spent"])),
        snapshot(8.0),
        control(9.0, "worker", "_close"),
        control(10.0, "parent", "_close"),
        snapshot(11.0),
        task(12.0, "after close"),
        control(13.0, "parent", "_resume_external"),
        control(14.0, "parent", "_close"),
        control(15.0, "worker", "_control_status"),
    ]));
}

/// `test_cancellation_stays_terminal_even_while_ended`.
#[test]
fn cancellation_stays_terminal_even_while_ended() {
    run_both(&with_worker([
        at(3.0, "parent", "stop", json!(["failed", "unrecoverable"])),
        at(
            4.0,
            "parent",
            "stop",
            json!(["blocked", "a second verdict while ended"]),
        ),
        at(5.0, "parent", "stop", json!(["cancelled", "user gave up"])),
        snapshot(6.0),
        at(7.0, "parent", "stop", json!(["cancelled", "again"])),
        control(8.0, "parent", "_resume_external"),
        at(9.0, "parent", "_extend_deadline", json!([60])),
        at(10.0, "parent", "pause", json!(["hold"])),
    ]));
}

/// `test_a_closed_run_cannot_be_cancelled_over`, and a placeholder has
/// nothing to cancel.
#[test]
fn a_closed_run_cannot_be_cancelled_over() {
    run_both(&with_worker([
        at(3.0, "parent", "stop", json!(["failed", "broken"])),
        control(4.0, "parent", "_close"),
        at(5.0, "parent", "stop", json!(["cancelled", "too late"])),
        at(6.0, "other", "stop", json!(["cancelled", "not a member"])),
        snapshot(7.0),
    ]));
    run_both(&[
        step(
            "boot",
            "bootstrap_run",
            json!([1, "start-b", "/tmp/b.sock"]),
            NOW,
        ),
        step(
            "boot",
            "stop",
            json!(["cancelled", "nothing to cancel"]),
            NOW + 1.0,
        ),
        step(
            "boot",
            "stop",
            json!(["blocked", "nothing to end"]),
            NOW + 2.0,
        ),
        step("boot", "_control_status", json!([]), NOW + 3.0),
        step("supervisor", "_status", json!([]), NOW + 4.0),
    ]);
}

/// `test_deadline_extension_is_capped_at_seven_days_ahead`, with the
/// loosely typed seconds Python's `type(seconds) is int` refuses.
#[test]
fn deadline_extension_is_capped_at_seven_days_ahead() {
    run_both(&with_worker([
        at(3.0, "parent", "_extend_deadline", json!([604_800])),
        at(4.0, "parent", "_extend_deadline", json!([3_600])),
        snapshot(5.0),
        at(6.0, "parent", "_extend_deadline", json!([60.0])),
        at(7.0, "parent", "_extend_deadline", json!([true])),
        at(8.0, "parent", "_extend_deadline", json!(["60"])),
        at(9.0, "parent", "_extend_deadline", json!([null])),
        step_text("parent", "_extend_deadline", "[-0]", NOW + 10.0),
        step_text("parent", "_extend_deadline", "[1e1]", NOW + 11.0),
        at(12.0, "worker", "_extend_deadline", json!([60])),
        at(13.0, "parent", "_extend_deadline", json!({"seconds": 1})),
        snapshot(14.0),
    ]));
}

/// A paused run's grant counts from the later of its deadline and its
/// pause start; a resume at the instant of the pause (or with the clock
/// behind it) records the integer 0.
#[test]
fn extend_grants_from_the_later_of_deadline_and_pause_start() {
    run_both(&with_worker([
        at(10.0, "parent", "pause", json!(["hold"])),
        sql(&format!("UPDATE run SET deadline={}", NOW + 5.0)),
        at(20.0, "parent", "_extend_deadline", json!([60])),
        snapshot(21.0),
        sql(&format!("UPDATE run SET deadline={}", NOW + 500.0)),
        at(22.0, "parent", "_extend_deadline", json!([60])),
        snapshot(23.0),
        control(23.0, "parent", "_resume_external"),
        at(30.0, "parent", "pause", json!(["hold again"])),
        control(30.0, "parent", "_resume_external"),
        at(31.0, "parent", "pause", json!(["and again"])),
        control(29.5, "parent", "_resume_external"),
        snapshot(32.0),
    ]));
}

/// `test_budget_expiry_is_distinct_and_keeps_partial_progress`: expiry
/// pauses holding `budget-exhausted` and kills nobody; the resume is
/// blocked until the deadline is extended, and the grant is future time.
#[test]
fn budget_expiry_is_distinct_and_keeps_partial_progress() {
    run_both(&with_worker([
        task(3.0, "work"),
        sql(&format!("UPDATE run SET deadline={}", NOW + 4.0)),
        at(5.0, "worker", "claim", json!([1])),
        snapshot(6.0),
        control(7.0, "parent", "_control_status"),
        control(8.0, "parent", "_resume_external"),
        at(9.0, "parent", "_extend_deadline", json!([0])),
        at(10.0, "parent", "stop", json!(["blocked", "stuck"])),
        sql("UPDATE run SET deadline=1"),
        at(11.0, "parent", "_extend_deadline", json!([600])),
        control(12.0, "parent", "_resume_external"),
        snapshot(13.0),
        at(14.0, "worker", "claim", json!([1])),
    ]));
}

/// `test_cancellation_and_expiry_prevent_new_work`, and a stop that meets
/// an expiry first: the gate's first transaction ends the run, so the
/// stop finds it already paused.
#[test]
fn cancellation_and_expiry_prevent_new_work() {
    run_both(&with_worker([
        at(3.0, "parent", "stop", json!(["cancelled", "user request"])),
        task(4.0, "after cancellation"),
        at(5.0, "worker", "_admit", json!(["child", "r"])),
        snapshot(6.0),
    ]));
    run_both(&with_worker([
        sql(&format!("UPDATE run SET deadline={}", NOW + 3.0)),
        at(4.0, "parent", "stop", json!(["blocked", "late"])),
        at(5.0, "parent", "stop", json!(["budget-exhausted", "late"])),
        snapshot(6.0),
    ]));
}

/// `usage_report_on_a_fresh_board_creates_the_usage_tables_identically`:
/// the first report creates the two tables at the same `sqlite_master`
/// positions and no budget row; the receipt and the report read what a
/// later slice's writes leave, and a budget at its limit blocks a resume.
#[test]
fn usage_report_on_a_fresh_board_creates_the_usage_tables_identically() {
    run_both(&with_worker([
        control(3.0, "worker", "usage_report"),
        control(4.0, "parent", "usage_report"),
        control(5.0, "parent", "_control_status"),
    ]));
    let requests = (0..12)
        .map(|index| {
            let actor = if index % 3 == 0 { "worker" } else { "parent" };
            let cache = if index == 0 { "NULL".to_owned() } else { index.to_string() };
            format!(
                "INSERT INTO request_usage VALUES('r{index}','{actor}','{{\"request_id\": \"r{index}\", \"n\": {index}.5}}',{index},{},1,1,2,{cache},NULL);",
                index % 2
            )
        })
        .collect::<String>();
    run_both(&with_worker([
        control(3.0, "parent", "_control_status"),
        sql(&requests),
        sql(
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": 60, "strict_unknown": false, "warned": true}')"#,
        ),
        control(4.0, "parent", "usage_report"),
        at(5.0, "parent", "stop", json!(["blocked", "over budget"])),
        control(6.0, "parent", "_resume_external"),
        sql(
            r#"UPDATE usage_budget SET payload='{"token_limit": null, "strict_unknown": true, "warned": false}'"#,
        ),
        control(7.0, "parent", "_resume_external"),
    ]));
}

/// A coordinator whose harness was lost after its latest activation
/// (#1924) blocks the resume until it is activated again.
#[test]
fn a_lost_coordinator_blocks_the_resume() {
    let event = |action: &str| {
        sql(&format!(
            r#"INSERT INTO events(actor,time,action,detail) VALUES('supervisor',{NOW},'{action}','{{"member": "parent"}}')"#
        ))
    };
    run_both(&with_worker([
        at(3.0, "parent", "pause", json!(["hold"])),
        event("scope_unknown"),
        control(4.0, "parent", "_control_status"),
        control(5.0, "parent", "_resume_external"),
        event("activated"),
        event("scope_observed"),
        control(6.0, "parent", "_resume_external"),
    ]));
}

/// A loss is a `scope_unknown` id above the latest `activated` one, where
/// a member with no activation counts as activated at id 0 (#2318
/// mutation report): a loss event a hand edit stored at id 0 or below is
/// no loss, and one above it is.
#[test]
fn a_loss_event_at_id_zero_or_below_is_no_loss() {
    let event = |id: i64| {
        sql(&format!(
            r#"INSERT INTO events(id,actor,time,action,detail) VALUES({id},'supervisor',{NOW},'scope_unknown','{{"member": "parent"}}')"#
        ))
    };
    run_both(&with_worker([
        at(3.0, "parent", "pause", json!(["hold"])),
        sql("DELETE FROM events WHERE action='activated'"),
        event(0),
        control(4.0, "parent", "_control_status"),
        event(-1),
        control(5.0, "parent", "_control_status"),
        event(1_000),
        control(6.0, "parent", "_control_status"),
    ]));
}

/// The control generation is the latest `paused` or `resumed` event id,
/// whatever it is (#2318 final review): a `resumed` event a hand edit
/// stored at id -5, the only control event, makes it -5, as Python
/// answers it.
#[test]
fn a_control_event_below_id_zero_is_the_generation() {
    run_both(&with_worker([
        sql(&format!(
            r#"INSERT INTO events(id,actor,time,action,detail) VALUES(-5,'supervisor',{NOW},'resumed','{{"paused_seconds": 0, "outcome": null}}')"#
        )),
        control(3.0, "parent", "_control_status"),
    ]));
}

/// Loosely typed reasons and statuses, and members that are not the
/// coordinator, are refused as Python refuses them.
#[test]
fn control_arguments_are_checked_as_python_checks_them() {
    let long = "x".repeat(8_193);
    run_both(&with_worker([
        at(3.0, "parent", "pause", json!([5])),
        at(4.0, "parent", "pause", json!([" "])),
        at(5.0, "parent", "pause", json!([long])),
        at(6.0, "parent", "pause", json!([null])),
        at(7.0, "parent", "stop", json!([["blocked"], "r"])),
        at(8.0, "parent", "stop", json!([null, "r"])),
        at(9.0, "parent", "stop", json!(["succeeded", "r"])),
        at(10.0, "parent", "stop", json!(["blocked", 5])),
        at(11.0, "parent", "stop", json!([5, 5])),
        at(12.0, "worker", "pause", json!(["hold"])),
        at(13.0, "worker", "stop", json!(["blocked", "r"])),
        control(14.0, "worker", "_resume_external"),
        control(15.0, "stranger", "_control_status"),
        at(16.0, "parent", "pause", json!(["é \u{2028} ☃"])),
        control(17.0, "parent", "_control_status"),
        sql("UPDATE members SET status='dead' WHERE id='worker'"),
        control(18.0, "worker", "_control_status"),
        control(19.0, "worker", "usage_report"),
    ]));
}

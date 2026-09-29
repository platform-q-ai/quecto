//! The divergences of the read models (#2277; epic #2265 P3), named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here by the
//! test of the same name:
//!
//! - `outside_edited_task_columns`, as the pin table describes it, for
//!   `summary`'s counts too: a task status the board never writes is
//!   refused naming the record, where Python raises `KeyError`, and a
//!   dependency naming no task counts the task blocked (Python's `_task`
//!   raises `TypeError` reading it first).
//! - `integer_beyond_i64_is_refused` (#2277 review L1), as the pin table
//!   describes it, for `events`' cursor and `tasks`' offset.
//! - `outside_edited_loss_records`, as `swarm_board_diff_loose_loss.rs`
//!   describes it, for the owner liveness too: an owner's latest event
//!   time that is not a number is refused naming the record wherever an
//!   owned task makes the board read it. Python raises `TypeError` only
//!   where it subtracts the time, for an owned task on the page it answers
//!   (`task`, `tasks`, or `summary`'s first 50); its liveness watch reads
//!   such an owner as `unknown`, so a `summary` whose owned task is 51st or
//!   later answers, where the Rust board refuses (#2277 final review L3).
use serde_json::json;

use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_rust, sql, try_run_both};

fn task(
    offset: f64,
    member: &str,
) -> crate::swarm_board_diff_runs::swarm_board_diff::scenario::Step {
    at(
        offset,
        member,
        "task_create",
        json!(["t", "implement behavior", ["tests pass"], []]),
    )
}

/// A status the board never writes, or a dependency naming no task, met
/// by `summary`'s counts: Python raises `KeyError`; the Rust board refuses
/// the status and counts the task blocked.
#[test]
fn outside_edited_task_columns() {
    let weird = joined([
        task(3.0, "worker"),
        sql("UPDATE tasks SET status='weird'"),
        at(4.0, "parent", "summary", json!([])),
    ]);
    let difference = try_run_both(&weird, |_, _, _| {}).unwrap_err();
    assert!(
        difference.contains("Python raised KeyError: 'weird'"),
        "{difference}"
    );
    assert_eq!(
        run_rust(&weird),
        Outcome::Refused("the board's task status is not as the board writes it".to_owned())
    );
    let missing = joined([
        task(3.0, "worker"),
        sql("UPDATE tasks SET dependencies='[9]'"),
        at(4.0, "parent", "summary", json!([])),
    ]);
    let difference = try_run_both(&missing, |_, _, _| {}).unwrap_err();
    // `_task` reads the missing dependency first: `fetchone()[0]` of None.
    assert!(
        difference.contains("Python raised TypeError: 'NoneType' object is not subscriptable"),
        "{difference}"
    );
    let Outcome::Ok(summary) = run_rust(&missing) else {
        panic!("the summary answers");
    };
    assert_eq!(summary["counts"]["blocked"], json!(1));
}

/// An owner's latest event time an edit made text: Python subtracts it
/// and raises for an owned task on the page it answers, and answers a
/// summary whose owned task is past its first 50; the Rust board refuses
/// the record in both.
#[test]
fn outside_edited_loss_records() {
    let steps = joined([
        task(3.0, "worker"),
        at(4.0, "worker", "claim", json!([1])),
        sql("UPDATE events SET time='soon' WHERE actor='worker'"),
        at(5.0, "parent", "task", json!([1])),
    ]);
    let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
    assert!(
        difference.starts_with("step 6: task as parent")
            && difference.contains("Python raised TypeError"),
        "{difference}"
    );
    assert_eq!(
        run_rust(&steps),
        Outcome::Refused("the board's event time is not as the board writes it".to_owned())
    );
    // The owned task beyond the summary's first page of 50 (#2277 final
    // review L3): Python's liveness watch reads the owner as `unknown` and
    // its page holds no owned task, so its summary answers; the Rust
    // board's watch refuses the record.
    let beyond = joined(
        (1..=51)
            .map(|index| {
                at(
                    3.0 + f64::from(index) / 100.0,
                    "worker",
                    "task_create",
                    json!([
                        format!("t{index}"),
                        "implement behavior",
                        ["tests pass"],
                        []
                    ]),
                )
            })
            .chain([
                at(4.0, "worker", "claim", json!([51])),
                sql("UPDATE events SET time='soon' WHERE actor='worker'"),
                at(5.0, "parent", "summary", json!([])),
            ]),
    );
    let difference = try_run_both(&beyond, |_, _, _| {}).unwrap_err();
    assert!(
        difference.starts_with("step 56: summary as parent") && difference.contains("python Ok("),
        "{difference}"
    );
    assert_eq!(
        run_rust(&beyond),
        Outcome::Refused("the board's event time is not as the board writes it".to_owned())
    );
}

/// `events`' cursor and `tasks`' offset beyond i64 but within u64:
/// Python's `sqlite3` raises `OverflowError` binding it; the Rust board
/// refuses it as a store failure naming Python's parameter position.
#[test]
fn integer_beyond_i64_is_refused() {
    for (method, parameter) in [("events", 1), ("tasks", 2)] {
        let steps = joined([at(3.0, "parent", method, json!([u64::MAX, 5]))]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with(&format!("step 3: {method} as parent"))
                && difference.contains(
                    "Python raised OverflowError: Python int too large to convert to SQLite INTEGER"
                ),
            "{difference}"
        );
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused(format!(
                "coordination store unavailable or contended: Error binding parameter \
                 {parameter}: Python int too large to convert to SQLite INTEGER"
            )),
            "{method}"
        );
    }
}

//! Differential scenarios for hand edits of the `run` row and the control
//! records (#2270, #2273; epic #2265 P3): what a file edited outside the
//! board leaves, read back through the served methods. Each must match;
//! where the Rust board deliberately differs, the divergence is named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here (listed
//! in its `EXTERNAL_PINS`).
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_both, run_rust, sql, step};

/// A NULL `run.status` (#2270 review L2) reads as Python reads it:
/// `_snapshot` answers `status: null`, `_status` answers it too, and
/// `create` over it is refused as over any run not in setup.
#[test]
fn a_null_run_status_reads_as_python_reads_it() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("UPDATE run SET status=NULL"),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
        step("supervisor", "_status", json!([]), NOW + 2.0),
        step(
            "parent",
            "create_run",
            json!(["g", [], [{"id": "c", "kind": "review", "description": "d"}], 5, NOW + 3_600.0]),
            NOW + 3.0,
        ),
        sql("UPDATE run SET deadline=0.5"),
        step("parent", "_snapshot", json!([]), NOW + 4.0),
    ]);
}

/// A run with no row for `_status` to read, after one existed.
#[test]
fn status_after_the_run_row_is_deleted_is_identical() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("DELETE FROM run"),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        step("parent", "_snapshot", json!([]), NOW + 2.0),
    ]);
}

/// `outside_edited_control_records` (#2273), records only a file edited
/// outside the board holds: a pause record whose `started` is not a
/// number, or a budget payload that is not an object (or whose token
/// limit is not a count), is refused naming the record, where Python
/// raises a `TypeError` (or, for a list payload, answers a budget of the
/// totals alone); an event detail that is not an object names no member
/// in the loss scan, where Python raises an `AttributeError`.
#[test]
fn outside_edited_control_records() {
    let paused = |edit: &str| {
        run_rust(&[
            create(5),
            at(1.0, "parent", "pause", json!(["hold"])),
            at(2.0, "parent", "_control_status", json!([])),
            sql(edit),
            at(3.0, "parent", "_control_status", json!([])),
        ])
    };
    let refused = |record: &str| {
        Outcome::Refused(format!(
            "the board's {record} is not as the board writes it"
        ))
    };
    assert_eq!(
        paused(r#"UPDATE events SET detail='{"started": "soon"}' WHERE action='paused'"#),
        refused("pause record")
    );
    assert_eq!(
        paused("INSERT INTO usage_budget VALUES(1, '[]')"),
        refused("usage budget")
    );
    assert_eq!(
        paused(
            r#"INSERT INTO usage_budget VALUES(1, '{"token_limit": "10", "strict_unknown": false}')"#
        ),
        refused("usage budget")
    );
    let unnamed = paused(
        r#"INSERT INTO events(actor,time,action,detail) VALUES('x',1.0,'scope_unknown','["parent"]')"#,
    );
    let Outcome::Ok(receipt) = unnamed else {
        panic!("{unnamed:?}");
    };
    assert_eq!(receipt["resume_blockers"], json!([]), "{receipt}");
}

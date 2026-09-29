//! The divergences of loss and death recording (#2277; epic #2265 P3),
//! named in `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned
//! here by the test of the same name:
//!
//! - `unknown_member_status_is_not_alive`, as the pin table describes it,
//!   for `_quarantine` and `_confirmed_dead` too: a member whose status is
//!   unknown or NULL (only a hand edit writes one) is not alive, so it is
//!   already lost and its death is already confirmed, where Python's
//!   `status == 'dead'` checks go on to observe or record its loss and to
//!   confirm its death.
//! - `outside_edited_loss_records`: a loss observation (`scope_observed`)
//!   whose time is not a number where `_grace_elapsed` measures the grace
//!   from it (text, or NULL for the caller's own observation) is refused
//!   naming the record, where Python raises a `TypeError`; and a BLOB time
//!   in any loss observation is refused as a store failure, where Python
//!   passes over an observation of another member.
use serde_json::json;

use crate::swarm_board_diff_loss::quarantine;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_rust, sql, try_run_both};

const CONTENDED: &str = "coordination store unavailable or contended: ";

/// A member whose status is unknown or NULL: Python confirms its death
/// and observes its loss; the Rust board takes it as already dead, and
/// writes nothing.
#[test]
fn unknown_member_status_is_not_alive() {
    for status in ["'zombie'", "NULL"] {
        for (method, args) in [
            ("_confirmed_dead", json!(["z"])),
            ("_quarantine", json!(["z"])),
        ] {
            let steps = joined([
                at(3.0, "parent", "_admit", json!(["z", "res-z"])),
                sql(&format!("UPDATE members SET status={status} WHERE id='z'")),
                at(4.0, "parent", method, args),
            ]);
            let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
            assert!(
                difference.starts_with(&format!("step 5: {method} as parent"))
                    && difference.contains("boards differ"),
                "{status} {method}: {difference}"
            );
            assert_eq!(run_rust(&steps), Outcome::Ok(json!(null)), "{method}");
        }
    }
}

/// A loss observation whose time an edit replaced: Python subtracts what
/// it finds and raises, or passes over another member's; the Rust board
/// refuses the record.
#[test]
fn outside_edited_loss_records() {
    let observed = |edit: &str| {
        joined([
            quarantine(3.0, "parent", json!("worker")),
            sql(edit),
            quarantine(13.0, "parent", json!("worker")),
        ])
    };
    let unmeasurable =
        Outcome::Refused("the board's loss observation is not as the board writes it".to_owned());
    for (edit, python) in [
        (
            "UPDATE events SET time='soon' WHERE action='scope_observed'",
            "Python raised TypeError: unsupported operand type(s) for -: 'float' and 'str'",
        ),
        (
            "UPDATE events SET time=NULL WHERE action='scope_observed'",
            "Python raised TypeError: unsupported operand type(s) for -: 'float' and 'NoneType'",
        ),
    ] {
        let steps = observed(edit);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 5: _quarantine as parent") && difference.contains(python),
            "{edit}: {difference}"
        );
        assert_eq!(run_rust(&steps), unmeasurable, "{edit}");
    }
    let blob = observed(
        r#"INSERT INTO events(actor,time,action,detail) VALUES('x',x'00','scope_observed','{"member":"other"}')"#,
    );
    let difference = try_run_both(&blob, |_, _, _| {}).unwrap_err();
    assert!(
        difference.starts_with("step 5: _quarantine as parent")
            && difference.contains("python Ok(Null)"),
        "{difference}"
    );
    let outcome = run_rust(&blob);
    assert!(
        matches!(&outcome, Outcome::Refused(text) if text.starts_with(CONTENDED)),
        "{outcome:?}"
    );
}

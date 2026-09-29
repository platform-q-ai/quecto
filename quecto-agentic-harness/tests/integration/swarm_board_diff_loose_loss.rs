//! The divergences of loss and death recording (#2277; epic #2265 P3),
//! named in `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned
//! here by the test of the same name (and, for a pin in `src`, listed in
//! [`SLICE_PINS`]):
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
//!   passes over an observation of another member. An observation whose
//!   detail is not an object (#2277 review L2) names no member, so the
//!   caller observes afresh, where Python's `.get` raises an
//!   `AttributeError`; one whose detail is not JSON text, or NULL (#2277
//!   final review L1), is refused as a store failure naming the column,
//!   where Python's `json.loads` raises a `JSONDecodeError` or a
//!   `TypeError`. A launcher that is not text (a BLOB) is no launcher, so
//!   the member's loss is recorded at once, where Python compares the
//!   bytes with the caller and asks whether that launcher is lost.
//! - `integer_beyond_i64_is_refused` (#2277 review L3), as the pin table
//!   describes it, for `_quarantine`'s and `_confirmed_dead`'s member.
use serde_json::json;

use crate::swarm_board_diff_loose::PERMITTED_DIVERGENCES;
use crate::swarm_board_diff_loss::quarantine;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_rust, sql, try_run_both};

/// The divergences this file pins also pinned in `src`: the name, the
/// test file's source and the pinning test in it.
const SLICE_PINS: [(&str, &str, &str); 2] = [
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/quarantine_member_tests.rs"),
        "an_unknown_member_status_is_already_lost",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/confirm_member_dead_tests.rs"),
        "an_unknown_member_status_is_already_dead",
    ),
];

const CONTENDED: &str = "coordination store unavailable or contended: ";

#[test]
fn this_slices_pins_in_src_exist() {
    for (name, source, test) in SLICE_PINS {
        assert!(PERMITTED_DIVERGENCES.contains(&name), "{name}");
        assert!(source.contains(&format!("fn {test}()")), "{test}");
    }
}

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
    // A detail that is not an object: Python's `.get` raises; the Rust
    // board reads it as naming no member, so the caller observes afresh
    // and the grace runs from now.
    let listed = observed("UPDATE events SET detail='[1]' WHERE action='scope_observed'");
    let difference = try_run_both(&listed, |_, _, _| {}).unwrap_err();
    assert!(
        difference.starts_with("step 5: _quarantine as parent")
            && difference
                .contains("Python raised AttributeError: 'list' object has no attribute 'get'"),
        "{difference}"
    );
    assert_eq!(run_rust(&listed), Outcome::Ok(json!(null)));
    // A detail that is not JSON text, or NULL (#2277 final review L1):
    // Python's `json.loads` raises; the Rust board refuses the column as a
    // store failure.
    for (edit, python, failure) in [
        (
            "UPDATE events SET detail='x' WHERE action='scope_observed'",
            "Python raised JSONDecodeError: Expecting value: line 1 column 1 (char 0)",
            "Conversion error from type Text at index: 2, Expecting value: line 1 column 1 (char 0)",
        ),
        (
            "UPDATE events SET detail=NULL WHERE action='scope_observed'",
            "Python raised TypeError: the JSON object must be str, bytes or bytearray, not NoneType",
            "Invalid column type Null at index: 2, name: detail",
        ),
    ] {
        let steps = observed(edit);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 5: _quarantine as parent") && difference.contains(python),
            "{edit}: {difference}"
        );
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused(format!("{CONTENDED}{failure}")),
            "{edit}"
        );
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
    // A BLOB launcher: Python observes the loss and waits out the grace;
    // the Rust board has no launcher to wait for, and records it at once.
    let launcher = joined([
        sql("UPDATE members SET launcher=x'00' WHERE id='worker'"),
        quarantine(3.0, "parent", json!("worker")),
        at(4.0, "parent", "_control_status", json!([])),
    ]);
    let difference = try_run_both(&launcher, |_, _, _| {}).unwrap_err();
    assert!(
        difference.starts_with("step 4: _quarantine as parent")
            && difference.contains("boards differ"),
        "{difference}"
    );
    let Outcome::Ok(receipt) = run_rust(&launcher) else {
        panic!("the receipt answers");
    };
    assert_eq!(receipt["outcome"], json!("failed"));
}

/// A member beyond i64 but within u64: Python's `sqlite3` raises
/// `OverflowError` binding it; the Rust board refuses it as a store
/// failure naming Python's parameter position.
#[test]
fn integer_beyond_i64_is_refused() {
    let beyond = json!(9_223_372_036_854_775_808_u64);
    for method in ["_quarantine", "_confirmed_dead"] {
        let steps = joined([at(3.0, "parent", method, json!([beyond]))]);
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
                "{CONTENDED}Error binding parameter 1: \
                 Python int too large to convert to SQLite INTEGER"
            )),
            "{method}"
        );
    }
}

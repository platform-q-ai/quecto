//! The divergences of durable messages (#2276; epic #2265 P3), named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here by the
//! test of the same name (or, for a name pinned in `src`, listed in
//! [`SLICE_PINS`]):
//!
//! - `integer_beyond_i64_is_refused`, as the pin table describes it, for a
//!   message id too (`withdraw`'s and `ack`'s, and `send`'s `supersedes`),
//!   `send`'s `recipient` and `inbox`'s `include_consumed`: Python raises
//!   `OverflowError` binding it; the Rust board refuses it naming parameter
//!   1 (parameter 2 for `include_consumed`).
//! - `unknown_member_status_is_not_alive`, as the pin table describes it,
//!   for `send` too: a recipient whose status is unknown or NULL is out of
//!   the swarm, where Python's `status == 'dead'` check sends to it.
//!   Pinned here and by [`SLICE_PINS`], which also lists the pins in `src`
//!   of the methods the pin table names for it.
//! - `outside_edited_messages`: a BLOB (only a hand edit writes one) in a
//!   `messages` column the op reads is refused as a store failure, where
//!   Python answers with bytes: `inbox` then cannot write its JSON, and
//!   `withdraw` or a superseding `send` takes a BLOB sender as another
//!   member's (`only your own message can be …`).
use serde_json::json;

use crate::swarm_board_diff_loose::PERMITTED_DIVERGENCES;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::{inbox, joined, send};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_rust, sql, try_run_both};

/// The divergences this file pins also pinned in `src`: the name, the
/// test file's source and the pinning test in it.
// Also holds src pins unrelated to messages: the checker requires them here.
const SLICE_PINS: [(&str, &str, &str); 5] = [
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/send_message_tests.rs"),
        "an_unknown_recipient_status_is_out_of_the_swarm",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/domain/swarm/policy_tests.rs"),
        "unknown_statuses_found_in_a_file_are_refused_affirmatively",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/domain/swarm/policy_null_status_tests.rs"),
        "a_null_member_status_is_not_alive",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/activate_member_tests.rs"),
        "an_unknown_member_status_is_not_activated",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/record_member_launch_tests.rs"),
        "an_unknown_member_status_records_no_launch",
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

/// A message id, `send`'s recipient or `inbox`'s `include_consumed` beyond
/// i64 but within u64: Python raises `OverflowError` where the Rust board
/// refuses it naming Python's parameter position.
#[test]
fn integer_beyond_i64_is_refused() {
    let beyond = json!(u64::MAX);
    for (method, args, parameter) in [
        ("withdraw", json!([beyond]), 1),
        ("ack", json!([beyond]), 1),
        ("send", json!(["s", "parent", "x", null, beyond]), 1),
        ("send", json!(["s", beyond, "x"]), 1),
        ("inbox", json!([beyond]), 2),
    ] {
        let steps = joined([
            send(3.0, "worker", "a", "parent", "one"),
            at(4.0, "worker", method, args),
        ]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with(&format!("step 4: {method} as worker"))
                && difference.contains(
                    "Python raised OverflowError: Python int too large to convert to SQLite INTEGER"
                ),
            "{difference}"
        );
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused(format!(
                "{CONTENDED}Error binding parameter {parameter}: \
                 Python int too large to convert to SQLite INTEGER"
            )),
            "{method} {parameter}"
        );
    }
}

/// A recipient whose status is unknown or NULL (only a hand edit writes
/// one): Python's `status == 'dead'` check sends to it, where the Rust
/// board refuses it as out of the swarm.
#[test]
fn unknown_member_status_is_not_alive() {
    for status in ["'zombie'", "NULL"] {
        let steps = joined([
            at(3.0, "parent", "_admit", json!(["z", "res-z"])),
            sql(&format!("UPDATE members SET status={status} WHERE id='z'")),
            send(4.0, "worker", "a", "z", "hi"),
        ]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 5: send as worker")
                && difference.contains(
                    r#"python Ok(Object {"id": Number(1), "status": String("accepted")})"#
                )
                && difference.contains(r#"rust   Refused("unknown or out-of-swarm recipient")"#),
            "{status}: {difference}"
        );
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused("unknown or out-of-swarm recipient".to_owned()),
            "{status}"
        );
    }
}

/// A BLOB in a message's column: Python cannot write the inbox holding it
/// and takes a BLOB sender as another member's; the Rust board refuses
/// each as a store failure.
#[test]
fn outside_edited_messages() {
    for (edit, probe, python) in [
        (
            "UPDATE messages SET body=x'00'",
            inbox(5.0, "parent", json!(false)),
            "Python raised unwritable result",
        ),
        (
            "UPDATE messages SET sender=x'00'",
            at(5.0, "worker", "withdraw", json!([1])),
            r#"python Refused("only your own message can be withdrawn")"#,
        ),
    ] {
        let steps = joined([send(3.0, "worker", "a", "parent", "one"), sql(edit), probe]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 5: ") && difference.contains(python),
            "{edit}: {difference}"
        );
        let outcome = run_rust(&steps);
        assert!(
            matches!(&outcome, Outcome::Refused(text) if text.starts_with(CONTENDED)),
            "{edit}: {outcome:?}"
        );
    }
}

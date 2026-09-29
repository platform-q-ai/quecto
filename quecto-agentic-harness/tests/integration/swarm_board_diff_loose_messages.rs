//! The divergences of durable messages (#2276; epic #2265 P3), named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here by the
//! test of the same name (or, for a name pinned in `src`, listed in
//! [`SLICE_PINS`]):
//!
//! - `integer_beyond_i64_is_refused`, as the pin table describes it, for a
//!   message id too (`withdraw`'s and `ack`'s, and `send`'s `supersedes`):
//!   Python takes it as an `int` of at least 1 and raises `OverflowError`
//!   binding it; the Rust board refuses it naming parameter 1.
//! - `unknown_member_status_is_not_alive`, as the pin table describes it,
//!   for `send` too: a recipient whose status is unknown or NULL is out of
//!   the swarm, where Python's `status == 'dead'` check sends to it.
//!   Pinned by `send_message_tests`.
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

/// This slice's divergences pinned in `src`: the name, the test file's
/// source and the pinning test in it.
const SLICE_PINS: [(&str, &str, &str); 1] = [(
    "unknown_member_status_is_not_alive",
    include_str!("../../src/application/swarm/use_cases/send_message_tests.rs"),
    "an_unknown_recipient_status_is_out_of_the_swarm",
)];

const CONTENDED: &str = "coordination store unavailable or contended: ";

#[test]
fn this_slices_pins_in_src_exist() {
    for (name, source, test) in SLICE_PINS {
        assert!(PERMITTED_DIVERGENCES.contains(&name), "{name}");
        assert!(source.contains(&format!("fn {test}()")), "{test}");
    }
}

/// A message id beyond i64 but within u64: Python raises `OverflowError`
/// where the Rust board refuses it.
#[test]
fn integer_beyond_i64_is_refused() {
    let beyond = json!(u64::MAX);
    for (method, args) in [
        ("withdraw", json!([beyond])),
        ("ack", json!([beyond])),
        ("send", json!(["s", "parent", "x", null, beyond])),
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
                "{CONTENDED}Error binding parameter 1: \
                 Python int too large to convert to SQLite INTEGER"
            )),
            "{method}"
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

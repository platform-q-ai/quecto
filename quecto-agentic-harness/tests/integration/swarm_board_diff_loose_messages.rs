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
//! - `wake_target_sort_error_order`: when the targets a wake op judges mix
//!   a member name with `None` (a NULL coordinator, only a hand edit writes
//!   one) or a number (a recipient `send` stored as given, e.g. `5` for the
//!   member `'5'`), Python's `sorted(targets)` raises a `TypeError` whose
//!   operand order follows the set's iteration order, which the hash seed
//!   decides; the Rust board refuses the op as a store failure with one
//!   fixed text (`domain::swarm::notification`'s `sort_error`).
//! - `outside_edited_wake_records`: an event detail the wake ops read that
//!   is not JSON text, or a `wake_cursors` row whose event is not an
//!   integer (only a hand edit writes either), is refused as a store
//!   failure. Python raises for a detail (`JSONDecodeError`) and for a
//!   TEXT or NULL cursor (`TypeError`), but compares and binds a REAL
//!   cursor as it is: after `('parent', 2.5)`, `_accept_wake(4)` claims
//!   past it and answers `true`, where Rust refuses reading it (`Invalid
//!   column type Real`). A stored dependency list is read as
//!   `outside_edited_task_columns` reads it, keeping its integer entries.
//! - `arguments_beyond_a_serde_value`, for `_accept_wake` too (pinned in
//!   `swarm_board_diff_loose.rs`): a generation above u64 is ahead of the
//!   board in Python; Rust refuses the argument text.
use serde_json::json;

use crate::swarm_board_diff_loose::PERMITTED_DIVERGENCES;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::{inbox, joined, send};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_rust, sql, try_run_both};
use crate::swarm_board_diff_wakes::{accept, hints};

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

/// A number beside a name (`send` stores the recipient `5` as given, and
/// the member `'5'` makes it deliverable), and `None` beside a name (a
/// NULL coordinator woken by evidence): Python's `TypeError` names the
/// two types in the order its set iterates, the Rust board in one order.
/// The sender's claim (`_notifications`) and the receiver's
/// (`_accept_wake`, judging the same events with the empty actor) meet it.
#[test]
fn wake_target_sort_error_order() {
    let numbered = |probe: Step| {
        joined([
            at(2.1, "parent", "_admit", json!(["5", "res-5"])),
            at(
                2.2,
                "parent",
                "_activate",
                json!(["5", "res-5", 5, "f", null]),
            ),
            at(3.0, "worker", "send", json!(["n", 5, "to five"])),
            send(4.0, "worker", "p", "parent", "to parent"),
            probe,
        ])
    };
    let nobody = joined([
        send(3.0, "worker", "p", "parent", "to parent"),
        sql("UPDATE run SET coordinator=NULL"),
        sql("INSERT INTO events(actor,time,action,detail) VALUES('worker',0,'evidence','{}')"),
        hints(5.0, "worker"),
    ]);
    for (steps, rust) in [
        (numbered(hints(5.0, "worker")), "'int' and 'str'"),
        (numbered(accept(5.0, "parent", json!(7))), "'int' and 'str'"),
        (nobody, "'str' and 'NoneType'"),
    ] {
        let last = steps.len() - 1;
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with(&format!("step {last}: "))
                && difference
                    .contains("Python raised TypeError: '<' not supported between instances of '"),
            "{difference}"
        );
        assert_eq!(
            run_rust(&steps),
            Outcome::Refused(format!("'<' not supported between instances of {rust}"))
        );
    }
}

/// An event detail that is not JSON, and a wake cursor that is text:
/// Python raises, the Rust board refuses as a store failure. A REAL wake
/// cursor Python compares and binds as it is (`4 <= 2.5` is false, so it
/// claims past it and answers), where the Rust board refuses reading it.
#[test]
fn outside_edited_wake_records() {
    let cursor = |value: &str| {
        format!(
            "CREATE TABLE wake_cursors (actor TEXT PRIMARY KEY, event INTEGER);
             INSERT INTO wake_cursors VALUES('parent',{value})"
        )
    };
    for (edit, probe, python, rust) in [
        (
            "INSERT INTO events(actor,time,action,detail) VALUES('worker',0,'amended','x')"
                .to_owned(),
            hints(5.0, "worker"),
            "Python raised JSONDecodeError",
            "",
        ),
        (
            cursor("'abc'"),
            accept(5.0, "parent", json!(4)),
            "Python raised TypeError",
            "Invalid column type Text",
        ),
        (
            cursor("NULL"),
            accept(5.0, "parent", json!(4)),
            "Python raised TypeError",
            "Invalid column type Null",
        ),
        (
            cursor("2.5"),
            accept(5.0, "parent", json!(4)),
            "python Ok(Bool(true))",
            "Invalid column type Real",
        ),
    ] {
        let steps = joined([send(3.0, "worker", "a", "parent", "one"), sql(&edit), probe]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 5: ") && difference.contains(python),
            "{edit}: {difference}"
        );
        let outcome = run_rust(&steps);
        assert!(
            matches!(&outcome, Outcome::Refused(text)
                if text.starts_with(CONTENDED) && text.contains(rust)),
            "{edit}: {outcome:?}"
        );
    }
}

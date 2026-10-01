//! Differential scenarios (#2276, epic #2265): durable messages (`send`,
//! `withdraw`, `inbox` and `ack`) on the Rust board against the Python
//! board's answers frozen in its golden fixtures (#2283), compared after
//! every step by result, refusal text and logical database dump. Ported
//! from the deleted Python suite `tests/swarm_helpers_test.py`, plus the loosely typed arguments
//! Python accepts (epic P3). The wake-notification steps of the ported
//! tests (`_notifications`, `_accept_wake`) are the rest of #2276.
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_golden, run_rust, sql};
use crate::swarm_board_diff_runs::{assert_changed_answer_refused, harness_self_test_tamper};

/// A running run of five coordinated by `parent`, with `worker` live.
pub(crate) fn joined(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
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

/// `member` sends `body` to `recipient` as request `request` at `offset`.
pub(crate) fn send(offset: f64, member: &str, request: &str, recipient: &str, body: &str) -> Step {
    at(offset, member, "send", json!([request, recipient, body]))
}

pub(crate) fn inbox(offset: f64, member: &str, include_consumed: Value) -> Step {
    at(offset, member, "inbox", json!([include_consumed]))
}

/// `count` accepted messages from `worker` to `parent`, as `send` writes
/// them, without their events: the inbox cap without a hundred steps.
fn filled(count: u32) -> Step {
    sql(&format!(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{count})
         INSERT INTO messages(sender,recipient,body,status) SELECT 'worker','parent','hello','accepted' FROM n"
    ))
}

/// `test_messages_are_durable_bounded_and_idempotent`: a send replays by
/// request id, the recipient reads it and acknowledges it, an unknown
/// recipient and a body over 8192 bytes are refused, a full inbox refuses
/// the hundred-and-first, and superseding one of the hundred makes room.
#[test]
fn messages_are_durable_bounded_and_idempotent() {
    run_golden(&joined([
        send(3.0, "worker", "m1", "parent", "blocked on schema"),
        send(4.0, "worker", "m1", "parent", "blocked on schema"),
        inbox(5.0, "parent", json!(false)),
        at(6.0, "parent", "ack", json!([1])),
        inbox(7.0, "parent", json!(false)),
        inbox(8.0, "parent", json!(true)),
        send(9.0, "worker", "bad", "unknown", "hello"),
        send(10.0, "worker", "bad", "parent", &"x".repeat(8193)),
        send(11.0, "worker", "edge", "parent", &"x".repeat(8192)),
        filled(98),
        send(12.0, "worker", "fill", "parent", "hello"),
        send(13.0, "worker", "overflow", "parent", "hello"),
        at(
            14.0,
            "worker",
            "send",
            json!(["replace", "parent", "newer", null, 2]),
        ),
        inbox(15.0, "parent", json!(false)),
    ]));
}

/// `test_messages_carry_a_revision_and_can_be_superseded_or_withdrawn`:
/// the inbox shows the revision and what a message supersedes, the audit
/// shows the superseded one, a replay answers the original receipt and a
/// changed field is a mismatch; only your own unread message to the same
/// recipient can be superseded, by an integer id; an empty revision is
/// refused; acknowledging a superseded message changes nothing.
#[test]
fn messages_carry_a_revision_and_can_be_superseded_or_withdrawn() {
    run_golden(&revisions());
}

/// The steps of [`messages_carry_a_revision_and_can_be_superseded_or_withdrawn`]:
/// `ack` of the superseded message 1 at `NOW + 17`, of message 2 at `NOW + 19`.
fn revisions() -> Vec<Step> {
    let revised = |offset, request: &str, body: &str, revision: &str, supersedes: Value| {
        at(
            offset,
            "worker",
            "send",
            json!([request, "parent", body, revision, supersedes]),
        )
    };
    joined([
        revised(3.0, "r1", "review head one", "abc1", Value::Null),
        revised(4.0, "r2", "review head two", "abc2", json!(1)),
        inbox(5.0, "parent", json!(false)),
        inbox(6.0, "parent", json!(true)),
        revised(7.0, "r2", "review head two", "abc2", json!(1)),
        revised(8.0, "r2", "review head two", "abc3", json!(1)),
        revised(9.0, "bad-1", "x", "abc4", json!(1)),
        revised(10.0, "bad-999", "x", "abc4", json!(999)),
        at(
            11.0,
            "parent",
            "send",
            json!(["bad-2", "parent", "x", null, 2]),
        ),
        send(12.0, "worker", "self", "worker", "note to self"),
        at(
            13.0,
            "worker",
            "send",
            json!({"request": "cross", "recipient": "parent", "body": "x", "supersedes": 3}),
        ),
        at(
            14.0,
            "worker",
            "send",
            json!(["bad-type", "parent", "x", null, "2"]),
        ),
        inbox(15.0, "parent", json!(false)),
        revised(16.0, "bad-rev", "x", "", Value::Null),
        at(17.0, "parent", "ack", json!([1])),
        inbox(18.0, "parent", json!(true)),
        at(19.0, "parent", "ack", json!([2])),
        inbox(20.0, "parent", json!(false)),
    ])
}

/// `test_plain_sends_replay_request_keys_recorded_before_message_revisions`:
/// a plain send's ledger key is `['send', recipient, body]`, so a row an
/// older build recorded replays, and one naming a revision is a mismatch.
#[test]
fn plain_sends_replay_request_keys_recorded_before_message_revisions() {
    run_golden(&joined([
        send(3.0, "worker", "legacy", "parent", "hello"),
        sql(
            r#"UPDATE requests SET payload='["send","parent","hello"]' WHERE actor='worker' AND request='legacy'"#,
        ),
        send(4.0, "worker", "legacy", "parent", "hello"),
        at(
            5.0,
            "worker",
            "send",
            json!(["legacy", "parent", "hello", "abc1"]),
        ),
        at(
            6.0,
            "worker",
            "send",
            json!(["revised", "parent", "hello", null, null]),
        ),
        at(7.0, "worker", "send", json!(["revised", "parent", "hello"])),
    ]));
}

/// `test_withdraw_and_ack_take_only_message_ids`: text, a boolean, zero,
/// null and a float are refused before the board is read, and only the
/// sender withdraws.
#[test]
fn withdraw_and_ack_take_only_message_ids() {
    let mut steps = joined([send(3.0, "worker", "w", "parent", "hello")]);
    for (index, bad) in [json!("1"), json!(true), json!(0), json!(null), json!(1.0)]
        .into_iter()
        .enumerate()
    {
        let offset = 4.0 + f64::from(u8::try_from(index).unwrap());
        steps.push(at(offset, "worker", "withdraw", json!([bad])));
        steps.push(at(offset + 0.5, "parent", "ack", json!([bad])));
    }
    steps.extend([
        inbox(10.0, "parent", json!(false)),
        at(11.0, "parent", "withdraw", json!([1])),
        at(12.0, "worker", "withdraw", json!({"message_id": 99})),
        at(13.0, "worker", "ack", json!([1])),
        at(14.0, "parent", "ack", json!([99])),
    ]);
    run_golden(&steps);
}

/// `test_a_withdrawn_message_leaves_the_inbox_and_wakes_nobody` (its
/// messages half): the withdrawn message leaves the inbox, stays in the
/// audit, a repeated withdrawal is a no-op, only the sender withdraws, and
/// an acknowledgment cannot revive it.
#[test]
fn a_withdrawn_message_leaves_the_inbox() {
    run_golden(&withdrawn());
}

/// The steps of [`a_withdrawn_message_leaves_the_inbox`]: the sender's
/// `withdraw` is at `NOW + 4`.
fn withdrawn() -> Vec<Step> {
    joined([
        at(
            3.0,
            "worker",
            "send",
            json!(["w1", "parent", "never mind", "abc1"]),
        ),
        at(4.0, "worker", "withdraw", json!([1])),
        inbox(5.0, "parent", json!(false)),
        inbox(6.0, "parent", json!(true)),
        at(7.0, "worker", "withdraw", json!([1])),
        at(8.0, "parent", "withdraw", json!([1])),
        at(9.0, "parent", "ack", json!([1])),
        inbox(10.0, "parent", json!(true)),
        at(
            11.0,
            "worker",
            "send",
            json!(["w2", "parent", "again", null, 1]),
        ),
    ])
}

/// #2394 round-1 review M1/L2: `ack` and `withdraw`'s changed answers name
/// the message the argument binds to, and a change the board holds.
#[test]
fn harness_self_test_checks_ack_and_withdraw_answers() {
    let steps = revisions();
    let difference = harness_self_test_tamper(&steps, "ack", 19.0, |answer| {
        answer.insert("message_id".to_owned(), json!(1));
    });
    assert_changed_answer_refused(
        difference,
        "message_id is not the message the argument names",
    );
    let difference = harness_self_test_tamper(&steps, "ack", 17.0, |answer| {
        answer.insert("changed".to_owned(), json!(true));
    });
    assert_changed_answer_refused(difference, "status is not consumed");
    let steps = withdrawn();
    let difference = harness_self_test_tamper(&steps, "withdraw", 4.0, |answer| {
        answer.insert("message_id".to_owned(), json!(2));
    });
    assert_changed_answer_refused(
        difference,
        "message_id is not the message the argument names",
    );
    let difference = harness_self_test_tamper(&steps, "withdraw", 4.0, |answer| {
        answer.insert("changed".to_owned(), json!("yes"));
    });
    assert_changed_answer_refused(difference, "changed is no bool");
}

/// #2394 final review L-1: `changed` must say whether the step changed
/// the message's status, read before and after it. A real change answered
/// `changed: false` (ack of message 2 at `NOW + 19`, the first withdrawal
/// at `NOW + 4`) is a difference.
#[test]
fn harness_self_test_checks_the_changed_flag_of_ack_and_withdraw() {
    for (steps, method, offset) in [(revisions(), "ack", 19.0), (withdrawn(), "withdraw", 4.0)] {
        let difference = harness_self_test_tamper(&steps, method, offset, |answer| {
            answer.insert("changed".to_owned(), json!(false));
        });
        assert_changed_answer_refused(
            difference,
            "changed is not whether the step changed the status: \"accepted\" before",
        );
    }
}

/// Python binds `include_consumed` into `(status='accepted' OR ?)`
/// untyped: SQLite's truth of the bound value decides, a NULL keeps only
/// accepted messages, and a list or an object is refused naming
/// parameter 2.
#[test]
fn inbox_include_consumed_true_is_bound_as_a_sql_parameter_identically() {
    let mut steps = joined([
        send(3.0, "worker", "a", "parent", "one"),
        send(4.0, "worker", "b", "parent", "two"),
        at(5.0, "parent", "ack", json!([1])),
    ]);
    for (index, include_consumed) in [
        json!(true),
        json!(false),
        json!(1),
        json!(0),
        json!(-2),
        json!(0.5),
        json!(0.0),
        json!("1"),
        json!("0"),
        json!("abc"),
        json!(" 2x"),
        json!(""),
        json!(null),
        json!([1]),
        json!({"a": 1}),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 6.0 + f64::from(u8::try_from(index).unwrap());
        steps.push(inbox(offset, "parent", include_consumed));
    }
    steps.push(at(30.0, "parent", "inbox", json!({})));
    run_golden(&steps);
}

/// The arguments `send` checks itself, before the operation gate: the
/// body, a revision other than null, and `supersedes` other than null
/// (an integer of at least 1, never a boolean); then, inside it, the
/// request id; and a recipient Python binds untyped (`5` finds the member
/// `'5'` through TEXT affinity, a list is refused naming parameter 1).
#[test]
fn send_checks_its_arguments_as_python_does() {
    let mut steps = joined([
        at(3.0, "parent", "_admit", json!(["5", "res-5"])),
        at(
            4.0,
            "parent",
            "_activate",
            json!(["5", "res-5", 12, "t", null]),
        ),
    ]);
    for (index, args) in [
        json!(["q", "parent", 5]),
        json!(["q", "parent", "   "]),
        json!(["q", "parent", null]),
        json!(["q", "parent", "x", 5]),
        json!(["q", "parent", "x", "   "]),
        json!(["q", "parent", "x", "r".repeat(257)]),
        json!(["q", "parent", "x", "r".repeat(256)]),
        json!(["q2", "parent", "x", null, true]),
        json!(["q2", "parent", "x", null, 0]),
        json!(["q2", "parent", "x", null, -1]),
        json!(["q2", "parent", "x", null, 1.0]),
        json!([5, "parent", "x"]),
        json!(["", "parent", "x"]),
        json!(["r".repeat(129), "parent", "x"]),
        json!([["q"], "parent", "x"]),
        json!(["q3", 5, "to five"]),
        json!(["q4", [5], "x"]),
        json!(["q5", null, "x"]),
        json!(["q6", {"a": 1}, "x"]),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 5.0 + f64::from(u8::try_from(index).unwrap());
        steps.push(at(offset, "worker", "send", args));
    }
    steps.extend([
        inbox(40.0, "5", json!(true)),
        inbox(41.0, "parent", json!(true)),
    ]);
    run_golden(&steps);
}

/// A recipient must be a member whose status is not `dead`: a reserved
/// member takes messages, a dead one is out of the swarm; a message to a
/// recipient that later died is still withdrawn and acknowledged.
#[test]
fn a_dead_recipient_is_out_of_the_swarm() {
    run_golden(&joined([
        at(3.0, "parent", "_admit", json!(["later", "res-l"])),
        send(4.0, "worker", "a", "later", "when you start"),
        send(5.0, "parent", "b", "worker", "hello"),
        sql("UPDATE members SET status='dead' WHERE id='worker'"),
        send(6.0, "parent", "c", "worker", "gone"),
        at(7.0, "parent", "withdraw", json!([2])),
        inbox(8.0, "later", json!(true)),
    ]));
}

/// A message's stored status is interpolated as Python's `str()` writes
/// it when it is neither accepted nor the withdrawal's own: a NULL is
/// `None`, a number its digits.
#[test]
fn a_retired_message_names_its_stored_status() {
    let mut steps = joined([
        send(3.0, "worker", "a", "parent", "one"),
        send(4.0, "worker", "b", "parent", "two"),
        send(5.0, "worker", "c", "parent", "three"),
        sql("UPDATE messages SET status=NULL WHERE id=1"),
        sql("UPDATE messages SET status=7 WHERE id=2"),
        sql("UPDATE messages SET status='consumed' WHERE id=3"),
    ]);
    for (index, id) in [1, 2, 3].into_iter().enumerate() {
        let offset = 6.0 + f64::from(u8::try_from(index).unwrap());
        steps.push(at(offset, "worker", "withdraw", json!([id])));
        steps.push(at(
            offset + 0.5,
            "worker",
            "send",
            json!([format!("s{id}"), "parent", "x", null, id]),
        ));
        steps.push(at(offset + 0.7, "parent", "ack", json!([id])));
    }
    run_golden(&steps);
}

/// `send` needs a running run (`authorize(active=True)`): on a paused run
/// it is refused with Python's own text and the board is left as it was,
/// where `withdraw` and `ack`, bookkeeping, still act (#2316's paused-run
/// table). Each op is one the unpaused board grants.
#[test]
fn every_mutating_message_op_on_a_paused_run() {
    let setup = joined([
        send(3.0, "worker", "a", "parent", "one"),
        send(4.0, "worker", "b", "parent", "two"),
    ]);
    let ops = [
        (
            "worker",
            "send",
            json!(["c", "parent", "three"]),
            Some("run is paused; no new work permitted"),
        ),
        ("worker", "withdraw", json!([1]), None),
        ("parent", "ack", json!([2]), None),
    ];
    let paused = sql("UPDATE run SET status='paused'");
    for (member, method, args, refusal) in ops {
        let op = at(9.0, member, method, args);
        let granted = [setup.clone(), vec![op.clone()]].concat();
        assert!(
            matches!(run_rust(&granted), Outcome::Ok(_)),
            "{method} is granted on the running run"
        );
        let steps = [
            setup.clone(),
            vec![paused.clone(), op, inbox(10.0, "parent", json!(true))],
        ]
        .concat();
        run_golden(&steps);
        let outcome = run_rust(&steps[..steps.len() - 1]);
        match refusal {
            Some(refusal) => assert!(
                matches!(&outcome, Outcome::Refused(text) if text == refusal),
                "{method}: {outcome:?}"
            ),
            None => assert!(matches!(outcome, Outcome::Ok(_)), "{method}: {outcome:?}"),
        }
    }
}

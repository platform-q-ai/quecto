//! Differential scenarios (#2277, epic #2265): the loss ops' checks,
//! beside `swarm_board_diff_loss.rs` so each file stays within 750 lines.
//! The gate and the exit kind, the member bound as Python binds it (epic
//! P3), `_lose_coordinator` in every run state, the paused-run table and
//! a loss on the setup placeholder, compared as there.
use serde_json::{Value, json};

use crate::swarm_board_diff_loss::{
    GRACE, answer, dead, held, quarantine, refusal, snapshot, state,
};
use crate::swarm_board_diff_membership::{HOUR, at, create};
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_golden, sql, step};

/// `_lose_coordinator` in every run state: a setup placeholder fails, a
/// running run or an outcome-less pause is lost as a failed pause, and a
/// run that holds an outcome or has ended is left alone; each answers the
/// run's columns and whether it recorded the loss.
#[test]
fn losing_the_coordinator_in_every_run_state() {
    let setup = [
        step("parent", "bootstrap_run", json!([1, "s", null]), NOW),
        at(1.0, "parent", "_lose_coordinator", json!([])),
        at(2.0, "parent", "_lose_coordinator", json!([])),
    ];
    run_golden(&setup);
    assert_eq!(answer(&setup, 1)["status"], json!("failed"));
    assert_eq!(answer(&setup, 2)["lost"], json!(false));
    for (name, before, lost, status) in [
        ("running", vec![], true, "paused"),
        (
            "paused",
            vec![at(3.0, "parent", "pause", json!(["hold"]))],
            true,
            "paused",
        ),
        (
            "held",
            vec![at(3.0, "parent", "stop", json!(["blocked", "why"]))],
            false,
            "paused",
        ),
        (
            "cancelled",
            vec![at(3.0, "parent", "stop", json!(["cancelled", "why"]))],
            false,
            "cancelled",
        ),
        (
            "closed",
            vec![
                at(3.0, "parent", "stop", json!(["blocked", "why"])),
                at(3.5, "parent", "_close", json!([])),
            ],
            false,
            "blocked",
        ),
    ] {
        let mut steps = joined(before);
        steps.push(at(4.0, "parent", "_lose_coordinator", json!([])));
        steps.push(at(5.0, "worker", "_lose_coordinator", json!([])));
        run_golden(&steps);
        let receipt = answer(&steps, steps.len() - 2);
        assert_eq!(
            (&receipt["lost"], &receipt["status"]),
            (&json!(lost), &json!(status)),
            "{name}: {receipt}"
        );
        assert_eq!(
            receipt.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id", "status", "outcome", "deadline", "coordinator", "lost"],
            "{name}"
        );
    }
}

/// The loss ops pass the operation gate as the harness does
/// (`active=False`): a stranger, or a member whose death is confirmed, is
/// refused; an exit kind is checked before the gate; an expired run ends
/// by its deadline first and is then lost as a held pause.
#[test]
fn the_loss_ops_are_gated_and_checked_as_python_does() {
    let mut steps = joined([
        quarantine(3.0, "stranger", json!("worker")),
        at(4.0, "stranger", "_confirmed_dead", json!(["worker"])),
        at(5.0, "stranger", "_confirmed_dead", json!(["worker", 1])),
        at(6.0, "stranger", "_lose_coordinator", json!([])),
        dead(7.0, "parent", "worker"),
        quarantine(8.0, "worker", json!("parent")),
        dead(9.0, "worker", "parent"),
        at(10.0, "worker", "_lose_coordinator", json!([])),
        dead(11.0, "parent", "worker"),
        quarantine(12.0, "parent", json!("worker")),
    ]);
    for exit in [json!("Orderly"), json!(null), json!(["abrupt"]), json!(0)] {
        steps.push(at(
            13.0,
            "parent",
            "_confirmed_dead",
            json!(["worker", exit]),
        ));
    }
    steps.push(at(
        14.0,
        "parent",
        "_confirmed_dead",
        json!({"exit": "abrupt", "member": "stranger"}),
    ));
    steps.push(at(HOUR + 1.0, "parent", "_lose_coordinator", json!([])));
    run_golden(&steps);
    let unknown = "invoking member is unknown or death confirmed";
    for index in [3, 4, 6, 8, 9, 10] {
        assert_eq!(refusal(&steps, index), unknown, "step {index}");
    }
    assert_eq!(refusal(&steps, 5), "exit kind must be orderly or abrupt");
    let expired = answer(&steps, steps.len() - 1);
    assert_eq!(
        (&expired["status"], &expired["outcome"], &expired["lost"]),
        (&json!("paused"), &json!("budget-exhausted"), &json!(false))
    );
}

/// Epic P3: the member is bound as Python binds it and compared by `==`.
/// `5` finds the member `'5'` (TEXT affinity) and is recorded as `5`,
/// which the loss scan then tells apart from `"5"`; `5.0` binds as a REAL
/// and finds no member; `null` finds nobody; a list or an object is
/// refused with Python's binding text; and `true` finds the member `'1'`.
#[test]
fn loss_ops_bind_the_member_as_python_does() {
    let mut steps = joined([
        at(3.0, "parent", "_admit", json!(["5", "res-5"])),
        at(
            3.1,
            "parent",
            "_activate",
            json!([5, "res-5", 12, "s", null]),
        ),
        at(3.2, "parent", "_admit", json!(["1", "res-1"])),
        at(
            3.3,
            "parent",
            "_activate",
            json!(["1", "res-1", 13, "s", null]),
        ),
    ]);
    steps.extend([
        quarantine(4.0, "parent", json!(5)),
        quarantine(4.0 + GRACE, "parent", json!(5)),
        snapshot(20.0, "parent"),
        at(21.0, "parent", "_resume_external", json!([])),
        quarantine(22.0, "parent", json!(5.0)),
        quarantine(22.0, "parent", json!("5")),
        quarantine(22.0 + GRACE, "parent", json!("5")),
        snapshot(35.0, "parent"),
        at(40.0, "parent", "_resume_external", json!([])),
        quarantine(41.0, "parent", Value::Null),
        quarantine(42.0, "parent", json!([5])),
        at(43.0, "parent", "_confirmed_dead", json!([[5]])),
        at(44.0, "parent", "_confirmed_dead", json!([true, "abrupt"])),
        at(45.0, "parent", "_confirmed_dead", json!([5.0])),
        at(46.0, "parent", "_confirmed_dead", json!([null])),
        at(46.5, "parent", "_confirmed_dead", json!([5])),
        snapshot(47.0, "parent"),
        quarantine(48.0, "parent", json!({"member": 5})),
    ]);
    run_golden(&steps);
    assert_eq!(state(&answer(&steps, 9)), held("failed"));
    assert_eq!(state(&answer(&steps, 14)), held("failed"));
    let binding = |kind: &str| {
        format!(
            "coordination store unavailable or contended: \
             Error binding parameter 1: type '{kind}' is not supported"
        )
    };
    assert_eq!(refusal(&steps, 17), binding("list"));
    assert_eq!(refusal(&steps, 18), binding("list"));
    assert_eq!(refusal(&steps, 24), binding("dict"));
    let members = answer(&steps, 23)["members"].clone();
    let status = |id: &str| {
        members
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["id"] == json!(id))
            .map(|member| member["status"].clone())
    };
    assert_eq!(status("5"), Some(json!("dead")));
    assert_eq!(status("1"), Some(json!("dead")));
}

/// Every loss op on a paused run, and on a run paused by an edit with no
/// pause record: each answers as Python does (the ops are
/// `active=False`, so a pause refuses none of them).
#[test]
fn every_loss_op_on_a_paused_run() {
    let paused = sql("UPDATE run SET status='paused'");
    for op in [
        quarantine(9.0, "parent", json!("worker")),
        quarantine(9.0, "worker", json!("parent")),
        dead(9.0, "parent", "worker"),
        dead(9.0, "worker", "parent"),
        at(9.0, "parent", "_lose_coordinator", json!([])),
    ] {
        let steps = joined([paused.clone(), op.clone(), op.clone()]);
        run_golden(&steps);
        let steps = joined([at(3.0, "parent", "pause", json!(["hold"])), op.clone(), op]);
        run_golden(&steps);
    }
}

/// A board with a run created over the setup placeholder: the coordinator
/// is launcher-less, and a loss recorded there fails the placeholder.
#[test]
fn a_loss_on_the_setup_placeholder_fails_it() {
    let steps = [
        step("parent", "bootstrap_run", json!([1, "s", null]), NOW),
        quarantine(1.0, "parent", json!("parent")),
        at(2.0, "supervisor", "_status", json!([])),
        create(5),
    ];
    run_golden(&steps);
    assert_eq!(answer(&steps, 2)["status"], json!("failed"));
}

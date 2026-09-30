//! Differential scenarios (#2271, epic #2265): membership admission,
//! activation, the launch records and the join, on the Rust board against
//! the Python board's answers frozen in its golden fixtures (#2283),
//! compared after every step by result, refusal text and logical database
//! dump.
use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_golden, sql, step};

pub(crate) const HOUR: f64 = 3_600.0;

/// `parent` creates a running run of `member_limit` members, an hour long.
pub(crate) fn create(member_limit: i64) -> Step {
    step(
        "parent",
        "create_run",
        json!({
            "goal": "admit members",
            "constraints": [],
            "criteria": [{"id": "t", "kind": "command", "description": "test"}],
            "member_limit": member_limit,
            "deadline": NOW + HOUR,
        }),
        NOW,
    )
}

pub(crate) fn at(offset: f64, member: &str, method: &str, args: Value) -> Step {
    step(member, method, args, NOW + offset)
}

pub(crate) fn snapshot(offset: f64) -> Step {
    at(offset, "parent", "_snapshot", json!([]))
}

#[test]
fn admit_activate_record_launch_are_identical() {
    run_golden(&[
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "res-1"])),
        // The same reservation again is an idempotent retry answering the row.
        at(2.0, "parent", "_admit", json!(["worker", "res-1"])),
        at(
            3.0,
            "parent",
            "_record_launch",
            json!(["worker", "res-1", 4242, "Mon Sep 28"]),
        ),
        at(
            4.0,
            "parent",
            "_record_launch",
            json!(["worker", "res-1", 4242, "Mon Sep 28"]),
        ),
        at(
            5.0,
            "parent",
            "_activate",
            json!(["worker", "res-1", 4242, "Mon Sep 28", "/w.sock"]),
        ),
        // The same process activating again is accepted.
        at(
            6.0,
            "parent",
            "_activate",
            json!(["worker", "res-1", 4242, "Mon Sep 28", null]),
        ),
        at(7.0, "worker", "_socket", json!(["/w2.sock"])),
        at(8.0, "worker", "_socket", json!({"socket": null})),
        // The member admits a nested member: it is that member's launcher.
        at(
            9.0,
            "worker",
            "_admit",
            json!({"member": "nested", "reservation": "res-2"}),
        ),
        at(
            10.0,
            "worker",
            "_activate",
            json!({"member": "nested", "reservation": "res-2", "pid": 7, "started": "t", "socket": "/n.sock"}),
        ),
        snapshot(11.0),
        at(12.0, "supervisor", "_status", json!([])),
        // A stranger may not act at all.
        at(13.0, "stranger", "_admit", json!(["other", "res-3"])),
        at(14.0, "stranger", "_socket", json!(["/s.sock"])),
    ]);
}

/// The join acts as the coordinator: it admits the joining member (the
/// coordinator is its launcher and the actor of both events), returns
/// early for the same live process, re-activates a known identity under
/// its own reservation and refuses any other.
#[test]
fn join_admits_as_the_coordinator_identically() {
    let steps = [
        step(
            "parent",
            "bootstrap_run",
            json!([1, "boot", "/p.sock"]),
            NOW,
        ),
        at(
            1.0,
            "worker",
            "bootstrap_join",
            json!([4242, "Mon", "/w.sock"]),
        ),
        at(
            2.0,
            "worker",
            "bootstrap_join",
            json!([4242, "Mon", "/w.sock", "anything"]),
        ),
        at(
            3.0,
            "helper",
            "bootstrap_join",
            json!({"pid": 5, "started": "t", "socket": null, "reservation": "given"}),
        ),
        at(
            4.0,
            "helper",
            "bootstrap_join",
            json!([6, "t", null, "other"]),
        ),
        at(5.0, "helper", "bootstrap_join", json!([6, "t", null])),
        at(
            6.0,
            "helper",
            "bootstrap_join",
            json!([6, "t", null, "given"]),
        ),
        at(7.0, "parent", "_admit", json!(["later", "later-r"])),
        at(
            8.0,
            "later",
            "bootstrap_join",
            json!([9, "t", null, "later-r"]),
        ),
        snapshot(9.0),
    ];
    run_golden(&steps);
    // The acceptance criterion, asserted outright: the joined member's
    // launcher is the coordinator, the acting member.
    let dir = tempfile::tempdir().unwrap();
    let board = RustBoard::open(&dir.path().join("swarm.sqlite"), dir.path());
    for step in &steps[..2] {
        let outcome = board
            .call_text(&step.member, &step.method, &step.args, step.now)
            .0;
        assert_eq!(outcome, Outcome::Ok(Value::Null), "{step:?}");
    }
    let Outcome::Ok(view) = board.call("parent", "_snapshot", &json!([]), NOW + 3.0) else {
        panic!("the snapshot reads");
    };
    let worker = view["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "worker")
        .unwrap();
    assert_eq!(worker["launcher"], json!("parent"), "{view}");
    assert_eq!(worker["status"], json!("live"), "{view}");
}

/// The deleted Python suite's `swarm_helpers_test.py::test_member_limit_accepts_upper_boundary`: 25
/// members, the coordinator included, are admitted, and the 26th is
/// refused with the same text.
#[test]
fn member_limit_accepts_upper_boundary_identically() {
    let mut steps = vec![create(25)];
    for index in 1..=25 {
        steps.push(at(
            f64::from(index),
            "parent",
            "_admit",
            json!([format!("member-{index:02}"), format!("reserve-{index:02}")]),
        ));
    }
    steps.push(at(30.0, "supervisor", "_status", json!([])));
    run_golden(&steps);
    let dir = tempfile::tempdir().unwrap();
    let board = RustBoard::open(&dir.path().join("swarm.sqlite"), dir.path());
    let outcomes: Vec<Outcome> = steps
        .iter()
        .map(|step| {
            board
                .call_text(&step.member, &step.method, &step.args, step.now)
                .0
        })
        .collect();
    assert!(
        outcomes[1..25]
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Ok(_)))
    );
    assert_eq!(
        outcomes[25],
        Outcome::Refused("swarm limit 25, current usage 25; reuse the existing pool".into())
    );
}

/// The sequential port of
/// `test_concurrent_admission_includes_idle_and_reserved_members`: a live
/// idle member and a reserved one count against the limit.
#[test]
fn idle_and_reserved_members_count_against_the_limit_identically() {
    let mut steps = vec![
        create(3),
        at(1.0, "parent", "_admit", json!(["worker", "reserve-worker"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "reserve-worker", 11, "w", null]),
        ),
    ];
    for index in 0..4 {
        steps.push(at(
            3.0 + f64::from(index),
            "worker",
            "_admit",
            json!([format!("child-{index}"), format!("reserve-{index}")]),
        ));
    }
    steps.push(at(8.0, "worker", "_admit", json!(["nested", "nested"])));
    steps.push(snapshot(9.0));
    run_golden(&steps);
}

/// A wrong reservation, a dead member and a different process are refused
/// with Python's texts by each method, and an expired run is ended before
/// activation is refused.
#[test]
fn stale_or_conflicting_launch_identity_is_refused_identically() {
    run_golden(&[
        create(6),
        at(1.0, "parent", "_admit", json!(["worker", "res-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "wrong", 1, "s", null]),
        ),
        at(
            3.0,
            "parent",
            "_record_launch",
            json!(["worker", "wrong", 1, "s"]),
        ),
        at(
            4.0,
            "parent",
            "_activate",
            json!(["stranger", "res-w", 1, "s", null]),
        ),
        at(
            5.0,
            "parent",
            "_record_launch",
            json!(["worker", "res-w", 1, "s"]),
        ),
        at(
            6.0,
            "parent",
            "_record_launch",
            json!(["worker", "res-w", 2, "s"]),
        ),
        at(
            7.0,
            "parent",
            "_record_launch",
            json!(["worker", "res-w", 1, "other"]),
        ),
        at(
            8.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 1, "s", "/w"]),
        ),
        at(
            9.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 2, "s", "/w"]),
        ),
        at(
            10.0,
            "worker",
            "bootstrap_join",
            json!([3, "s", null, "not-mine"]),
        ),
        // A released reservation is dead: every launch record refuses it,
        // and so does a new admission of the same identity.
        at(11.0, "parent", "_admit", json!(["gone", "res-g"])),
        at(12.0, "parent", "_release_unlaunched", json!(["gone"])),
        at(
            13.0,
            "parent",
            "_activate",
            json!(["gone", "res-g", 1, "s", null]),
        ),
        at(
            14.0,
            "parent",
            "_record_launch",
            json!(["gone", "res-g", 1, "s"]),
        ),
        at(15.0, "parent", "_admit", json!(["gone", "res-g"])),
        at(16.0, "gone", "_socket", json!(["/g"])),
        snapshot(17.0),
        // Admitted before the deadline, activated after it: the expiry
        // commits, then activation is refused.
        at(18.0, "parent", "_admit", json!(["late", "res-l"])),
        at(
            HOUR + 1.0,
            "parent",
            "_activate",
            json!(["late", "res-l", 5, "s", null]),
        ),
        at(HOUR + 2.0, "parent", "_admit", json!(["later", "res-2"])),
        snapshot(HOUR + 3.0),
    ]);
}

#[test]
fn release_unlaunched_only_for_reserved_members_without_pid() {
    run_golden(&[
        create(6),
        at(1.0, "parent", "_admit", json!(["launched", "res-a"])),
        at(
            2.0,
            "parent",
            "_record_launch",
            json!(["launched", "res-a", 7, "t"]),
        ),
        at(3.0, "parent", "_release_unlaunched", json!(["launched"])),
        at(4.0, "parent", "_admit", json!(["idle", "res-b"])),
        at(
            5.0,
            "parent",
            "_release_unlaunched",
            json!({"member": "idle"}),
        ),
        at(6.0, "parent", "_release_unlaunched", json!(["idle"])),
        at(7.0, "parent", "_release_unlaunched", json!(["parent"])),
        at(8.0, "parent", "_release_unlaunched", json!(["stranger"])),
        // A dead releaser is refused by the gate.
        at(9.0, "idle", "_release_unlaunched", json!(["launched"])),
        snapshot(10.0),
    ]);
}

#[test]
fn a_member_name_over_128_bytes_is_refused() {
    run_golden(&[
        create(6),
        at(1.0, "parent", "_admit", json!(["x".repeat(129), "r1"])),
        at(2.0, "parent", "_admit", json!(["é".repeat(65), "r2"])),
        at(3.0, "parent", "_admit", json!(["", "r3"])),
        at(4.0, "parent", "_admit", json!(["  \t", "r4"])),
        at(5.0, "parent", "_admit", json!([5, "r5"])),
        at(6.0, "parent", "_admit", json!(["x".repeat(128), "r6"])),
        at(7.0, "parent", "_admit", json!(["é".repeat(64), "r7"])),
        at(
            8.0,
            "x".repeat(129).as_str(),
            "bootstrap_join",
            json!([1, "s", null]),
        ),
        snapshot(9.0),
    ]);
}

/// A board edited outside to lose its coordinator: the join reads the
/// coordinator column alone and, acting as nobody, is refused by the gate.
#[test]
fn a_join_without_a_coordinator_is_refused_identically() {
    run_golden(&[
        step(
            "parent",
            "bootstrap_run",
            json!([1, "boot", "/p.sock"]),
            NOW,
        ),
        sql("UPDATE run SET coordinator=NULL"),
        at(1.0, "worker", "bootstrap_join", json!([2, "t", null])),
        sql("DELETE FROM run"),
        at(2.0, "worker", "bootstrap_join", json!([2, "t", null])),
    ]);
}

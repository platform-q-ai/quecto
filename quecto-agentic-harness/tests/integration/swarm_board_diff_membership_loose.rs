//! Differential scenarios for loosely typed membership arguments (#2271
//! round-1 review M1, M2; epic #2265 P3): `_admit`, `_activate`,
//! `_record_launch`, `_release_unlaunched`, `_socket` and the join bind
//! their member, reservation, pid, start time and socket as Python's
//! `sqlite3` binds them, and compare stored values with Python's `==`.
//! Each must match; a list or an object is refused by both with Python's
//! parameter position, and an integer beyond i64 stays the
//! `integer_beyond_i64_is_refused` divergence.
use serde_json::json;

use crate::swarm_board_diff_membership::{at, create, snapshot};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{run_both, sql, step};

/// `_admit` stores the reservation as given (TEXT affinity makes a number
/// its text), and a retry compares the stored value with Python's `==`:
/// `None == None`, but `'5' != 5` and `'1' != True`.
#[test]
fn admit_binds_the_reservation_as_python_does() {
    run_both(&[
        create(12),
        at(1.0, "parent", "_admit", json!(["a", null])),
        at(2.0, "parent", "_admit", json!(["a", null])),
        at(3.0, "parent", "_admit", json!(["a2", null])),
        at(4.0, "parent", "_admit", json!(["b", 5])),
        at(5.0, "parent", "_admit", json!(["b", 5])),
        at(6.0, "parent", "_admit", json!(["b", "5"])),
        // The reservation is UNIQUE: `5` is stored as '5'.
        at(7.0, "parent", "_admit", json!(["c", "5"])),
        at(8.0, "parent", "_admit", json!(["d", 5.5])),
        at(9.0, "parent", "_admit", json!(["e", true])),
        at(10.0, "parent", "_admit", json!(["e", 1])),
        at(11.0, "parent", "_admit", json!(["f", false])),
        at(12.0, "parent", "_admit", json!(["g", [1]])),
        at(13.0, "parent", "_admit", json!(["h", {"k": 1}])),
        at(
            14.0,
            "parent",
            "_admit",
            json!({"member": "i", "reservation": -0.0}),
        ),
        snapshot(15.0),
    ]);
}

/// `_activate` binds the pid, start time and socket as given (INTEGER
/// affinity converts numeric text and an integral float), finds the
/// member by a number through TEXT affinity, and compares the recorded
/// process with Python's `==` (`7 == 7.0`, `1 == True`, `7 != '7'`).
#[test]
fn activate_binds_the_launch_identity_as_python_does() {
    run_both(&[
        create(12),
        at(1.0, "parent", "_admit", json!(["w1", "r1"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["w1", "r1", "7", "s", null]),
        ),
        at(
            3.0,
            "parent",
            "_activate",
            json!(["w1", "r1", 7, "s", null]),
        ),
        at(
            4.0,
            "parent",
            "_activate",
            json!(["w1", "r1", 7.0, "s", null]),
        ),
        at(
            5.0,
            "parent",
            "_activate",
            json!(["w1", "r1", "7", "s", null]),
        ),
        at(
            6.0,
            "parent",
            "_activate",
            json!(["w1", "r1", [7], "s", null]),
        ),
        at(7.0, "parent", "_admit", json!(["w2", "r2"])),
        at(8.0, "parent", "_activate", json!(["w2", "r2", true, 5, 5])),
        at(
            9.0,
            "parent",
            "_activate",
            json!(["w2", "r2", 1, "5", null]),
        ),
        at(
            10.0,
            "parent",
            "_activate",
            json!(["w2", "r2", true, 5, null]),
        ),
        at(11.0, "parent", "_admit", json!(["w3", "r3"])),
        at(
            12.0,
            "parent",
            "_activate",
            json!(["w3", "r3", 7.5, 5.5, true]),
        ),
        at(13.0, "parent", "_admit", json!(["w4", null])),
        at(
            14.0,
            "parent",
            "_activate",
            json!(["w4", null, 8, "t", null]),
        ),
        at(15.0, "parent", "_admit", json!(["w5", 5])),
        at(16.0, "parent", "_activate", json!(["w5", 5, 9, "t", null])),
        at(
            17.0,
            "parent",
            "_activate",
            json!(["w5", "5", 9, "t", null]),
        ),
        at(18.0, "parent", "_admit", json!(["6", "r6"])),
        at(19.0, "parent", "_activate", json!([6, "r6", 10, "t", null])),
        at(20.0, "parent", "_admit", json!(["w7", "r7"])),
        at(
            21.0,
            "parent",
            "_activate",
            json!(["w7", "r7", [1], "t", null]),
        ),
        at(
            22.0,
            "parent",
            "_activate",
            json!(["w7", "r7", 1, {}, null]),
        ),
        at(
            23.0,
            "parent",
            "_activate",
            json!(["w7", "r7", 1, "t", [2]]),
        ),
        at(
            24.0,
            "parent",
            "_activate",
            json!([["w7"], "r7", 1, "t", null]),
        ),
        at(
            25.0,
            "parent",
            "_activate",
            json!([null, null, 1, "t", null]),
        ),
        snapshot(26.0),
    ]);
}

/// `_record_launch` finds the row by member and reservation in SQL (a
/// NULL reservation matches nothing; `5` matches '5' through TEXT
/// affinity) and compares a recorded process with Python's `==`.
#[test]
fn record_launch_binds_as_python_does() {
    run_both(&[
        create(8),
        at(1.0, "parent", "_admit", json!(["worker", "r1"])),
        at(
            2.0,
            "parent",
            "_record_launch",
            json!(["worker", null, 1, "s"]),
        ),
        at(3.0, "parent", "_admit", json!(["w5", 5])),
        at(4.0, "parent", "_record_launch", json!(["w5", 5, "7", 5])),
        at(5.0, "parent", "_record_launch", json!(["w5", "5", 7, "5"])),
        at(6.0, "parent", "_record_launch", json!(["w5", "5", 7.0, 5])),
        at(
            7.0,
            "parent",
            "_record_launch",
            json!(["w5", "5", true, "5"]),
        ),
        at(8.0, "parent", "_admit", json!(["5", "r-5"])),
        at(
            9.0,
            "parent",
            "_record_launch",
            json!([5, "r-5", 1.0, null]),
        ),
        at(
            10.0,
            "parent",
            "_record_launch",
            json!(["worker", "r1", [1], "t"]),
        ),
        at(
            11.0,
            "parent",
            "_record_launch",
            json!(["worker", [1], 1, "t"]),
        ),
        at(
            12.0,
            "parent",
            "_record_launch",
            json!(["worker", "r1", 2, {"t": 1}]),
        ),
        snapshot(13.0),
    ]);
}

/// `_release_unlaunched` finds the member by a number through TEXT
/// affinity (`5` is '5', `7.0` is '7.0'), and names it in the event as
/// given.
#[test]
fn release_unlaunched_binds_the_member_as_python_does() {
    run_both(&[
        create(8),
        at(1.0, "parent", "_admit", json!(["5", "r5"])),
        at(2.0, "parent", "_release_unlaunched", json!([5])),
        at(3.0, "parent", "_admit", json!(["7", "r7"])),
        at(4.0, "parent", "_release_unlaunched", json!([7.0])),
        at(5.0, "parent", "_admit", json!(["1", "r1"])),
        at(6.0, "parent", "_release_unlaunched", json!([true])),
        at(7.0, "parent", "_release_unlaunched", json!([null])),
        at(8.0, "parent", "_release_unlaunched", json!([[1]])),
        snapshot(9.0),
    ]);
}

/// `_socket` stores the socket as given (a number as its text).
#[test]
fn socket_binds_as_python_does() {
    run_both(&[
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "r"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "r", 3, "s", null]),
        ),
        at(3.0, "worker", "_socket", json!([5])),
        snapshot(4.0),
        at(5.0, "worker", "_socket", json!([true])),
        at(6.0, "worker", "_socket", json!([5.5])),
        snapshot(7.0),
        at(8.0, "worker", "_socket", json!([[1]])),
        at(9.0, "worker", "_socket", json!([{"a": 1}])),
        at(10.0, "worker", "_socket", json!([null])),
        snapshot(11.0),
    ]);
}

/// The join takes Python's `reservation or uuid4().hex` (any falsy value
/// draws a fresh one), compares the member's process and reservation with
/// Python's `==`, and binds what it passes on as given: a reservation
/// admitted as `5` is stored as '5', so the activation that follows finds
/// it stale after the admission committed.
#[test]
fn join_binds_as_python_does() {
    run_both(&[
        step(
            "parent",
            "bootstrap_run",
            json!([1, "boot", "/p.sock"]),
            NOW,
        ),
        at(
            1.0,
            "w",
            "bootstrap_join",
            json!([4242, "Mon", "/w.sock", 0]),
        ),
        at(2.0, "w", "bootstrap_join", json!(["4242", "Mon", null])),
        at(3.0, "w", "bootstrap_join", json!([4242.0, "Mon", null])),
        at(4.0, "x", "bootstrap_join", json!([1, "t", null, 5])),
        at(5.0, "x", "bootstrap_join", json!([1, "t", null, "5"])),
        at(6.0, "y", "bootstrap_join", json!([2, "t", 7, ""])),
        at(7.0, "z", "bootstrap_join", json!([3, "t", null, false])),
        at(8.0, "v", "bootstrap_join", json!([4, 4.5, null, 0.0])),
        at(9.0, "q", "bootstrap_join", json!([true, 5, null, [1]])),
        snapshot(10.0),
    ]);
}

/// A REAL pid of 2^63, which SQLite keeps as REAL because no INTEGER
/// holds it, is not the process `i64::MAX`: Python compares exactly
/// (#2271 round-1 review M2).
#[test]
fn a_stored_real_pid_of_two_to_the_63_is_not_i64_max() {
    run_both(&[
        create(5),
        at(1.0, "parent", "_admit", json!(["worker", "r"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "r", 1, "s", null]),
        ),
        sql("UPDATE members SET pid=9223372036854775807.0 WHERE id='worker'"),
        at(
            3.0,
            "parent",
            "_activate",
            json!(["worker", "r", i64::MAX, "s", null]),
        ),
        at(
            4.0,
            "parent",
            "_record_launch",
            json!(["worker", "r", i64::MAX, "s"]),
        ),
        snapshot(5.0),
    ]);
    run_both(&[
        step(
            "parent",
            "bootstrap_run",
            json!([1, "boot", "/p.sock"]),
            NOW,
        ),
        sql("UPDATE members SET pid=9223372036854775807.0 WHERE id='parent'"),
        at(
            1.0,
            "parent",
            "bootstrap_join",
            json!([i64::MAX, "boot", null]),
        ),
        snapshot(2.0),
    ]);
}

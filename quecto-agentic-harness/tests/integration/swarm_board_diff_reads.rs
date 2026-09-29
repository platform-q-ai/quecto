//! Differential scenarios (#2277, epic #2265): the read models every
//! member polls (`summary` with its cursor fast path and
//! `next_liveness_check_at`, `events` paging, `task` and `tasks` with
//! their owner's liveness, #1969) on the Python board and the Rust board,
//! compared after every step by result (key order and float values
//! included), refusal text and logical database dump. Ported from
//! `tests/swarm_helpers_test.py`. `joined` writes events 1 (`created`),
//! 2 (`reserved`) and 3 (`activated`); the first claim is token 3.
use serde_json::{Value, json};

use crate::swarm_board_diff_files::token;
use crate::swarm_board_diff_loss::{answer, dead, lose, refusal};
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_messages::{joined, send};
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_both};

/// `swarm_policy.OWNER_IDLE_AFTER`, in seconds.
pub(crate) const IDLE_AFTER: f64 = 300.0;

pub(crate) fn summary(offset: f64, member: &str, since: Value) -> Step {
    at(offset, member, "summary", json!([since]))
}

pub(crate) fn full(offset: f64, member: &str) -> Step {
    at(offset, member, "summary", json!([]))
}

pub(crate) fn task(offset: f64, member: &str, request: &str) -> Step {
    at(
        offset,
        member,
        "task_create",
        json!([request, "implement behavior", ["tests pass"], []]),
    )
}

fn view(offset: f64, member: &str, task: i64) -> Step {
    at(offset, member, "task", json!([task]))
}

fn page(offset: f64, member: &str, args: Value) -> Step {
    at(offset, member, "tasks", args)
}

const LIVENESS: [&str; 3] = ["owner_last_activity", "owner_state", "contact"];

/// `test_summary_cursor_avoids_unchanged_payload_and_history_cursor_is_validated`.
#[test]
fn summary_cursor_avoids_unchanged_payload_and_history_cursor_is_validated() {
    let mut steps = joined([
        full(3.0, "parent"),
        summary(4.0, "parent", json!(3)),
        task(5.0, "worker", "new change"),
        summary(6.0, "parent", json!(3)),
    ]);
    for invalid in [json!(-1), json!(true), json!("1"), json!(1.5)] {
        steps.push(at(7.0, "parent", "events", json!({"after": invalid})));
        steps.push(summary(7.5, "parent", invalid));
    }
    run_both(&steps);
    assert_eq!(answer(&steps, 3)["event_cursor"], json!(3));
    assert_eq!(
        answer(&steps, 4),
        json!({"unchanged": true, "event_cursor": 3, "status": "running", "next_liveness_check_at": null})
    );
    assert!(answer(&steps, 6).get("unchanged").is_none());
    assert_eq!(
        refusal(&steps, 7),
        "event page requires nonnegative cursor and limit 1 through 100"
    );
    assert_eq!(
        refusal(&steps, 8),
        "summary cursor must be a nonnegative integer"
    );
}

/// `test_default_summary_does_not_replay_historical_contract_payloads`.
#[test]
fn default_summary_does_not_replay_historical_contract_payloads() {
    let criteria = json!([{"id": "t", "kind": "command", "description": "test"}]);
    let mut steps = joined([]);
    for index in 0..12 {
        steps.push(at(
            3.0 + f64::from(index),
            "parent",
            "amend",
            json!([
                "admit members",
                ["x".repeat(6000)],
                criteria,
                format!("change {index}")
            ]),
        ));
    }
    steps.extend([
        full(20.0, "parent"),
        at(21.0, "parent", "events", json!({"after": 0, "limit": 2})),
        at(22.0, "parent", "events", json!({"after": 2, "limit": 2})),
        at(23.0, "parent", "events", json!({"after": 15})),
        at(24.0, "parent", "events", json!([])),
    ]);
    run_both(&steps);
    let summary = answer(&steps, 15);
    assert!(summary.to_string().len() < 20_000);
    assert!(summary.get("events").is_none());
    assert_eq!(summary["event_cursor"], json!(15));
    let first = answer(&steps, 16);
    assert_eq!(
        (
            first["events"].as_array().unwrap().len(),
            &first["cursor"],
            &first["has_more"]
        ),
        (2, &json!(2), &json!(true))
    );
    assert_eq!(answer(&steps, 17)["events"][0]["id"], json!(3));
    assert_eq!(
        answer(&steps, 18),
        json!({"events": [], "cursor": 15, "has_more": false})
    );
}

/// `test_summary_is_bounded_and_counts_include_later_pages`.
#[test]
fn summary_is_bounded_and_counts_include_later_pages() {
    let mut steps = joined([]);
    for index in 0..55 {
        steps.push(task(3.0, "worker", &format!("task-{index}")));
    }
    steps.extend([
        full(4.0, "parent"),
        page(5.0, "worker", json!({"offset": 50})),
        at(6.0, "worker", "file_owners", json!([])),
        page(7.0, "worker", json!({"limit": 1000})),
    ]);
    run_both(&steps);
    let summary = answer(&steps, 58);
    assert_eq!(summary["tasks"].as_array().map(Vec::len), Some(50));
    assert_eq!(
        (&summary["task_count"], &summary["counts"]["ready"]),
        (&json!(55), &json!(55))
    );
    assert_eq!(answer(&steps, 59).as_array().map(Vec::len), Some(5));
    assert_eq!(
        refusal(&steps, 61),
        "task page requires nonnegative offset and limit 1 through 100"
    );
}

/// `test_an_unclaimed_task_carries_no_owner_fields`.
#[test]
fn an_unclaimed_task_carries_no_owner_fields() {
    let steps = joined([
        task(3.0, "worker", "task"),
        view(4.0, "parent", 1),
        page(5.0, "parent", json!([])),
        full(6.0, "parent"),
    ]);
    run_both(&steps);
    for found in [
        answer(&steps, 4),
        answer(&steps, 5)[0].clone(),
        answer(&steps, 6)["tasks"][0].clone(),
    ] {
        assert_eq!(found["owner"], Value::Null);
        for field in LIVENESS {
            assert!(found.get(field).is_none(), "{field}: {found}");
        }
    }
}

/// `test_a_completed_task_keeps_its_owner_column_but_carries_no_liveness`.
#[test]
fn a_completed_task_keeps_its_owner_column_but_carries_no_liveness() {
    let steps = joined([
        task(3.0, "worker", "done"),
        task(3.1, "worker", "live"),
        at(4.0, "worker", "claim", json!([1])),
        at(
            4.1,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "tests.log", "revision": "r1"}]]),
        ),
        at(4.2, "parent", "verify_task", json!([1, token(3), "r1"])),
        at(4.3, "worker", "claim", json!([2])),
        page(5.0, "parent", json!([])),
    ]);
    run_both(&steps);
    let found = answer(&steps, 9);
    assert_eq!(
        (&found[0]["status"], &found[1]["status"], &found[0]["owner"]),
        (&json!("completed"), &json!("claimed"), &json!("worker"))
    );
    for field in LIVENESS {
        assert!(found[0].get(field).is_none(), "{field}");
    }
    assert_eq!(found[1]["owner_state"], json!("active"));
}

/// `test_a_claimed_task_names_its_active_owner_and_how_to_reach_it`.
#[test]
fn a_claimed_task_names_its_active_owner_and_how_to_reach_it() {
    let steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        view(5.0, "parent", 1),
        page(6.0, "worker", json!([])),
    ]);
    run_both(&steps);
    let found = answer(&steps, 5);
    assert_eq!(
        (
            &found["owner"],
            &found["owner_state"],
            &found["owner_last_activity"]
        ),
        (&json!("worker"), &json!("active"), &json!(1.0))
    );
    assert_eq!(
        found["contact"],
        json!("board.send(request, 'worker', body)")
    );
    assert!(found.get("recovery").is_none());
    assert_eq!(answer(&steps, 6)[0]["contact"], found["contact"]);
}

/// `test_a_quiet_owner_reads_as_idle_after_the_documented_threshold`: the
/// reader's clock decides, and reading writes nothing.
#[test]
fn a_quiet_owner_reads_as_idle_after_the_documented_threshold() {
    let base = 10.0;
    let steps = joined([
        task(3.0, "worker", "task"),
        at(3.5, "parent", "_extend_deadline", json!([3600])),
        at(4.0, "worker", "claim", json!([1])),
        at(
            base,
            "worker",
            "block",
            json!([1, token(3), "waiting on W2"]),
        ),
        view(base + IDLE_AFTER - 1.0, "parent", 1),
        view(base + IDLE_AFTER, "parent", 1),
        page(base + IDLE_AFTER, "parent", json!([])),
        full(base + IDLE_AFTER, "parent"),
        view(base + 2.0 * IDLE_AFTER, "parent", 1),
    ]);
    run_both(&steps);
    assert_eq!(answer(&steps, 7)["owner_state"], json!("active"));
    for found in [
        answer(&steps, 8),
        answer(&steps, 9)[0].clone(),
        answer(&steps, 10)["tasks"][0].clone(),
    ] {
        assert_eq!(
            (
                &found["status"],
                &found["owner_state"],
                &found["owner_last_activity"]
            ),
            (&json!("blocked"), &json!("idle"), &json!(IDLE_AFTER))
        );
        assert_eq!(
            found["contact"],
            json!("board.send(request, 'worker', body)")
        );
    }
    assert_eq!(
        answer(&steps, 11)["owner_last_activity"],
        json!(2.0 * IDLE_AFTER)
    );
}

/// `test_a_dead_owner_reads_as_dead_with_recovery_instead_of_a_contact`.
#[test]
fn a_dead_owner_reads_as_dead_with_recovery_instead_of_a_contact() {
    let steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        dead(5.0, "parent", "worker"),
        view(6.0, "parent", 1),
        send(7.0, "parent", "to-the-dead", "worker", "anyone there?"),
    ]);
    run_both(&steps);
    let found = answer(&steps, 6);
    assert_eq!(
        (
            &found["status"],
            &found["owner"],
            &found["owner_state"],
            &found["contact"]
        ),
        (
            &json!("blocked"),
            &json!("worker"),
            &json!("dead"),
            &Value::Null
        )
    );
    assert_eq!(
        found["recovery"],
        json!("recover(task) or revoke(task, reason)")
    );
    assert!(refusal(&steps, 7).contains("out-of-swarm recipient"));
}

/// `test_a_lost_owner_reads_as_lost_with_the_resume_note_and_no_contact`:
/// the loss record outranks the clock.
#[test]
fn a_lost_owner_reads_as_lost_with_the_resume_note_and_no_contact() {
    let mut steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
    ]);
    steps.extend(lose(5.0, "parent", "worker"));
    steps.extend([
        full(16.0, "parent"),
        view(17.0, "parent", 1),
        page(18.0, "parent", json!([])),
        view(17.0 + 2.0 * IDLE_AFTER, "parent", 1),
    ]);
    run_both(&steps);
    assert_eq!(answer(&steps, 7)["status"], json!("paused"));
    for found in [
        answer(&steps, 8),
        answer(&steps, 9)[0].clone(),
        answer(&steps, 10),
    ] {
        assert_eq!(
            (&found["owner"], &found["owner_state"], &found["contact"]),
            (&json!("worker"), &json!("lost"), &Value::Null)
        );
        let recovery = found["recovery"].as_str().unwrap();
        assert!(recovery.contains("resume the run") && recovery.contains("revoke(task, reason)"));
    }
}

/// `test_summary_cursor_reports_an_owner_turning_idle_by_clock_alone`:
/// an owner turning idle by the clock alone defeats the fast path; a
/// later board event moves the cursor and restores it.
#[test]
fn summary_cursor_reports_an_owner_turning_idle_by_clock_alone() {
    let base = 10.0;
    let steps = joined([
        task(3.0, "worker", "task"),
        at(3.5, "parent", "_extend_deadline", json!([3600])),
        at(base, "worker", "claim", json!([1])),
        full(base + IDLE_AFTER - 1.0, "parent"),
        summary(base + IDLE_AFTER - 1.0, "parent", json!(6)),
        summary(base + IDLE_AFTER, "parent", json!(6)),
        send(
            base + IDLE_AFTER + 5.0,
            "worker",
            "ping",
            "parent",
            "still here",
        ),
        full(base + IDLE_AFTER, "parent"),
        summary(base + IDLE_AFTER, "parent", json!(7)),
    ]);
    run_both(&steps);
    let first = answer(&steps, 6);
    let check = json!(crate::swarm_board_diff_runs::NOW + base + IDLE_AFTER);
    assert_eq!(
        (&first["event_cursor"], &first["next_liveness_check_at"]),
        (&json!(6), &check)
    );
    let again = answer(&steps, 7);
    assert_eq!(
        (&again["unchanged"], &again["next_liveness_check_at"]),
        (&json!(true), &check)
    );
    let crossed = answer(&steps, 8);
    assert!(crossed.get("unchanged").is_none());
    assert_eq!(
        (
            &crossed["event_cursor"],
            &crossed["tasks"][0]["owner_state"],
            &crossed["next_liveness_check_at"]
        ),
        (&json!(6), &json!("idle"), &Value::Null)
    );
    let moved = answer(&steps, 10);
    assert_eq!(moved["tasks"][0]["owner_state"], json!("active"));
    assert_eq!(moved["tasks"][0]["owner_last_activity"], json!(0.0));
    assert_eq!(answer(&steps, 11)["unchanged"], json!(true));
}

/// `test_status_and_summary_count_members_without_a_claim_and_dead_members`.
#[test]
fn status_and_summary_count_members_without_a_claim_and_dead_members() {
    let status = |offset: f64| at(offset, "supervisor", "_status", json!([]));
    let steps = joined([
        status(3.0),
        full(3.5, "parent"),
        task(4.0, "worker", "task"),
        at(4.5, "worker", "claim", json!([1])),
        status(5.0),
        at(
            5.5,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "tests.log", "revision": "r1"}]]),
        ),
        status(6.0),
        at(
            6.5,
            "parent",
            "_admit",
            json!(["reserved-only", "reservation-r"]),
        ),
        status(7.0),
        at(
            7.5,
            "parent",
            "_release_unlaunched",
            json!(["reserved-only"]),
        ),
        dead(8.0, "parent", "worker"),
        status(8.5),
        full(9.0, "parent"),
    ]);
    run_both(&steps);
    let counts = |value: Value| {
        (
            value["members_without_claim"].clone(),
            value["members_dead"].clone(),
        )
    };
    assert_eq!(counts(answer(&steps, 3)), (json!(1), json!(0)));
    assert_eq!(
        answer(&steps, 4)["counts"]["members_without_claim"],
        json!(1)
    );
    assert_eq!(counts(answer(&steps, 7)).0, json!(0));
    assert_eq!(counts(answer(&steps, 9)).0, json!(0));
    assert_eq!(counts(answer(&steps, 11)), (json!(1), json!(0)));
    assert_eq!(counts(answer(&steps, 14)), (json!(0), json!(2)));
    let summary = answer(&steps, 15)["counts"].clone();
    assert_eq!(
        (counts(summary.clone()), &summary["blocked"]),
        ((json!(0), json!(2)), &json!(1))
    );
}

/// `test_owner_liveness_is_one_grouped_events_scan_per_page`, as the
/// answer shows it (the scan count is pinned on the application's
/// counting double, `board_read_models_tests`).
#[test]
fn owner_liveness_is_one_grouped_events_scan_per_page() {
    let mut steps = joined([
        at(3.0, "parent", "_admit", json!(["other", "reservation-o"])),
        at(
            3.1,
            "parent",
            "_activate",
            json!(["other", "reservation-o", 12, "o", null]),
        ),
    ]);
    for index in 0..6_i64 {
        let offset = 4.0 + 0.1 * index as f64;
        steps.push(task(offset, "worker", &format!("task-{index}")));
        let claimant = if index % 2 == 1 { "worker" } else { "other" };
        steps.push(at(offset + 0.05, claimant, "claim", json!([index + 1])));
    }
    steps.push(page(5.0, "parent", json!([])));
    run_both(&steps);
    let states: Vec<Value> = answer(&steps, steps.len() - 1)
        .as_array()
        .unwrap()
        .iter()
        .map(|found| found["owner_state"].clone())
        .collect();
    assert_eq!(states, vec![json!("active"); 6]);
}

/// `test_notification_batch_carries_atomic_board_generation`, in full:
/// the batch's generation is the summary's event cursor.
#[test]
fn notification_batch_carries_atomic_board_generation() {
    let steps = joined([
        send(3.0, "parent", "wake-batch", "worker", "action"),
        at(4.0, "parent", "_notifications", json!([true])),
        full(5.0, "parent"),
        at(6.0, "parent", "_notifications", json!([true])),
    ]);
    run_both(&steps);
    let batch = answer(&steps, 4);
    assert_eq!(batch["generation"], answer(&steps, 5)["event_cursor"]);
    assert!(
        batch["members"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == json!("worker"))
    );
    assert_eq!(answer(&steps, 6)["members"], json!([]));
}

/// #2277 review M1: four owners claim at non-round times, and the
/// summary at `NOW + 200` answers each owner's quiet time as a float
/// whose shortest repr needs sixteen significant digits (Python's
/// `93.65000009536743`). Both answers are compared exactly: Python's is
/// read with `py_json`, whose float parse rounds correctly, where
/// `serde_json` without `float_roundtrip` misreads such a float by an
/// ulp and reports a difference the boards do not have.
#[test]
fn owner_liveness_floats_compare_exactly() {
    let mut steps = joined([]);
    for (index, owner) in ["w2", "w3", "w4"].into_iter().enumerate() {
        let offset = 3.0 + 0.1 * index as f64;
        let reservation = format!("res-{owner}");
        steps.push(at(offset, "parent", "_admit", json!([owner, reservation])));
        steps.push(at(
            offset + 0.05,
            "parent",
            "_activate",
            json!([owner, reservation, 20 + index, "o", null]),
        ));
    }
    let claims = [
        ("worker", 106.35),
        ("w2", 105.9),
        ("w3", 97.123_456_789),
        ("w4", 101.7),
    ];
    for (index, (owner, offset)) in claims.into_iter().enumerate() {
        steps.push(task(4.0 + index as f64, "worker", &format!("task-{index}")));
        steps.push(at(offset, owner, "claim", json!([index + 1])));
    }
    steps.push(full(200.0, "parent"));
    run_both(&steps);
    let summary = answer(&steps, steps.len() - 1);
    let quiet: Vec<f64> = summary["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|found| found["owner_last_activity"].as_f64().unwrap())
        .collect();
    assert_eq!(quiet.len(), 4, "{summary}");
    assert!(
        quiet.iter().any(|seconds| seconds.to_string().len() >= 17),
        "a quiet time needs every digit: {quiet:?}"
    );
}

/// #2277 review L2: a member id holding code points Unicode has not
/// assigned (U+0378, and U+E0080 beyond the basic plane) reaches its
/// task's `contact` escaped as Python's `repr()` escapes it. Neither is
/// assigned in Unicode 15.1 or 16.0, so the scenario holds under CI's
/// Python 3.13 as under 3.14, whose Unicode 16.0 table the board uses.
#[test]
fn an_unassigned_code_point_in_an_owner_id_is_escaped_as_python_does() {
    let owner = "w\u{0378}\u{e0080}";
    let steps = joined([
        at(3.0, "parent", "_admit", json!([owner, "res-u"])),
        at(
            3.1,
            "parent",
            "_activate",
            json!([owner, "res-u", 5, "u", null]),
        ),
        task(4.0, owner, "unassigned"),
        at(5.0, owner, "claim", json!([1])),
        view(6.0, "parent", 1),
        page(6.5, "parent", json!([])),
    ]);
    run_both(&steps);
    assert_eq!(
        answer(&steps, 7)["contact"],
        json!(r"board.send(request, 'w\u0378\U000e0080', body)")
    );
}

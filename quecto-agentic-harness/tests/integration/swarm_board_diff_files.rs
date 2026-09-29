//! Differential scenarios (#2275, epic #2265): file reservations, recovery,
//! revocation and the evidence success needs, on the Rust board against the
//! Python board's answers frozen in its golden fixtures (#2283), compared
//! after every step by result, refusal text and logical database dump
//! (`files` ordered by path). Ported from `tests/swarm_helpers_test.py`,
//! plus the loosely typed arguments Python accepts (epic P3).
//!
//! Each side's checkout is its own board directory; [`mkdir`] and
//! [`symlink`] shape both alike. A conflicting `reserve` here always meets
//! exactly one reserved path: Python names whichever its hash-seeded `set`
//! meets first, so a multi-conflict scenario would differ from Python
//! itself (the `multi_conflict_names_the_smallest_path` divergence, pinned
//! in `swarm_board_diff_loose_files.rs`).
//!
//! Death is set up by a direct `UPDATE members SET status='dead'`
//! ([`sql`]), not `_confirmed_dead`, which S12 ports: `recover` reads only
//! the owner's status, so the direct edit is what it gates on.
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, mkdir, run_golden, run_rust, sql, step, symlink,
};

/// The `n`th id the harness draws, as the counter writes it.
pub(crate) fn token(n: u64) -> String {
    format!("{n:032x}")
}

/// `member` admitted and activated by the parent at `offset`.
fn joined(offset: f64, member: &str) -> [Step; 2] {
    let reservation = format!("res-{member}");
    [
        at(offset, "parent", "_admit", json!([member, reservation])),
        at(
            offset + 0.1,
            "parent",
            "_activate",
            json!([member, reservation, 11, "t", null]),
        ),
    ]
}

/// A running run of five coordinated by `parent`, with `worker` live and
/// holding the claim of task 1 under [`token`]`(3)`.
pub(crate) fn claimed(more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    let mut steps = vec![create(5)];
    steps.extend(joined(1.0, "worker"));
    steps.extend([
        at(
            3.0,
            "worker",
            "task_create",
            json!(["task", "work", ["tests pass"]]),
        ),
        at(4.0, "worker", "claim", json!([1])),
    ]);
    steps.extend(more);
    steps
}

fn owners(offset: f64) -> Step {
    at(offset, "parent", "file_owners", json!([]))
}

fn raw(offset: f64) -> Step {
    at(offset, "parent", "task_raw", json!([1]))
}

fn dead(member: &str) -> Step {
    sql(&format!(
        "UPDATE members SET status='dead' WHERE id='{member}'"
    ))
}

/// `test_file_reservations_are_atomic_normalized_and_token_owned`: a path
/// is normalised, a set with one reserved path reserves nothing, only the
/// owner's current claim releases, and an escape is refused.
#[test]
fn file_reservations_are_atomic_normalized_and_token_owned() {
    run_golden(&claimed([
        at(
            5.0,
            "parent",
            "task_create",
            json!(["second", "other", ["pass"]]),
        ),
        at(6.0, "parent", "claim", json!([2])),
        at(
            7.0,
            "worker",
            "reserve",
            json!([1, token(3), ["src/../a.rs"]]),
        ),
        at(
            8.0,
            "parent",
            "reserve",
            json!([2, token(4), ["free.rs", "./a.rs"]]),
        ),
        owners(9.0),
        at(
            10.0,
            "worker",
            "release_files",
            json!([1, "stale", token(5)]),
        ),
        at(
            11.0,
            "worker",
            "release_files",
            json!([1, token(3), token(5)]),
        ),
        owners(12.0),
        at(
            13.0,
            "worker",
            "reserve",
            json!([1, token(3), ["../escape"]]),
        ),
    ]));
}

/// `test_symlink_alias_cannot_bypass_reservation`: a path through a
/// symlinked directory is reserved under its target.
#[test]
fn symlink_alias_cannot_bypass_reservation() {
    run_golden(&claimed([
        mkdir("real"),
        symlink("alias", "real"),
        at(5.0, "worker", "reserve", json!([1, token(3), ["real/new"]])),
        at(
            6.0,
            "parent",
            "task_create",
            json!(["b", "other", ["pass"]]),
        ),
        at(7.0, "parent", "claim", json!([2])),
        at(
            8.0,
            "parent",
            "reserve",
            json!([2, token(5), ["alias/new"]]),
        ),
        at(
            9.0,
            "parent",
            "reserve",
            json!([2, token(5), ["alias/../real/other", "missing/deep/x.rs"]]),
        ),
        owners(10.0),
    ]));
}

/// The paths Python's non-strict `resolve()` keeps, follows or refuses,
/// one reservation each (the `checkout_paths_tests` table).
#[test]
fn reserved_paths_resolve_as_python_resolves_them() {
    let mut more = vec![
        mkdir("real"),
        mkdir("src"),
        symlink("alias", "real"),
        symlink("loop", "loop2"),
        symlink("loop2", "loop"),
        symlink("up", ".."),
        symlink("rel", "src/../real"),
    ];
    let paths = [
        "a/../b.rs",
        "./x",
        "missing/deep/file.rs",
        ".",
        "src/",
        "loop/x",
        "rel/n",
        "missing/../a2.rs",
        "alias/../src/a.rs",
        "src//c.rs",
        "//x",
        "/etc/passwd",
        "up/escape",
        "a\u{0}b",
    ];
    for (index, path) in paths.into_iter().enumerate() {
        let offset = 5.0 + f64::from(u8::try_from(index).unwrap());
        more.push(at(
            offset,
            "worker",
            "reserve",
            json!([1, token(3), [path]]),
        ));
    }
    more.push(owners(30.0));
    run_golden(&claimed(more));
}

/// `test_resolved_blocker_resumes_original_claim_without_releasing_files`,
/// the file part: a blocked claim reserves, and unblocking keeps them.
#[test]
fn resolved_blocker_resumes_original_claim_without_releasing_files() {
    run_golden(&claimed([
        at(
            5.0,
            "worker",
            "block",
            json!([1, token(3), "awaiting approval"]),
        ),
        at(6.0, "worker", "reserve", json!([1, token(3), ["owned.rs"]])),
        at(
            7.0,
            "worker",
            "unblock",
            json!([1, token(3), "approval received"]),
        ),
        raw(8.0),
        owners(9.0),
        at(
            10.0,
            "parent",
            "unblock",
            json!([1, token(3), "not the owner"]),
        ),
        at(
            11.0,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "tests", "revision": "R1"}]]),
        ),
        at(
            12.0,
            "worker",
            "unblock",
            json!([1, token(3), "cannot reopen"]),
        ),
    ]));
}

/// `test_revoke_is_coordinator_only`.
#[test]
fn revoke_is_coordinator_only() {
    let mut more = joined(5.0, "other").to_vec();
    more.extend([
        at(6.0, "worker", "revoke", json!([1, "not mine to take"])),
        at(7.0, "other", "revoke", json!([1, "not mine to take"])),
        raw(8.0),
    ]);
    run_golden(&claimed(more));
}

/// `test_revoke_reopens_work_drops_reservations_and_tells_the_previous_owner`:
/// the task is ready again without its reservations, the audit names the
/// reason and previous owner, the previous owner's message is written, and
/// another member claims, submits and completes it.
#[test]
fn revoke_reopens_work_drops_reservations_and_tells_the_previous_owner() {
    let mut more = vec![
        at(5.0, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        at(
            6.0,
            "worker",
            "block",
            json!([1, token(3), "awaiting provider"]),
        ),
        at(7.0, "parent", "release", json!([1, token(3)])),
        at(8.0, "parent", "recover", json!([1])),
        at(
            9.0,
            "parent",
            "revoke",
            json!([1, "member suspended by provider"]),
        ),
        owners(10.0),
    ];
    more.extend(joined(11.0, "other"));
    more.extend([
        at(12.0, "other", "claim", json!([1])),
        at(
            13.0,
            "other",
            "submit",
            json!([1, token(5), [{"artifact": "a.log", "revision": "r2"}]]),
        ),
        at(14.0, "parent", "verify_task", json!([1, token(5), "r2"])),
        raw(15.0),
    ]);
    run_golden(&claimed(more));
}

/// `test_revoked_token_cannot_submit_release_reserve_or_verify`.
#[test]
fn revoked_token_cannot_submit_release_reserve_or_verify() {
    let evidence = json!([{"artifact": "a.log", "revision": "r1"}]);
    run_golden(&claimed([
        at(5.0, "worker", "submit", json!([1, token(3), evidence])),
        at(6.0, "parent", "revoke", json!([1, "stale submission"])),
        at(7.0, "worker", "submit", json!([1, token(3), evidence])),
        at(7.1, "worker", "release", json!([1, token(3)])),
        at(7.2, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        at(7.3, "worker", "block", json!([1, token(3), "x"])),
        at(7.4, "worker", "release_files", json!([1, token(3), "r"])),
        at(8.0, "parent", "verify_task", json!([1, token(3), "r1"])),
        raw(9.0),
    ]));
}

/// `test_revoke_is_idempotent_and_refuses_unclaimed_or_completed_work`:
/// revoking an unowned task answers it and records nothing; an empty
/// reason is refused; completed work is refused.
#[test]
fn revoke_is_idempotent_and_refuses_unclaimed_or_completed_work() {
    run_golden(&claimed([
        at(5.0, "parent", "revoke", json!([1, "first"])),
        at(6.0, "parent", "revoke", json!([1, "second"])),
        at(7.0, "parent", "revoke", json!([1, ""])),
        at(8.0, "worker", "claim", json!([1])),
        at(
            9.0,
            "worker",
            "submit",
            json!([1, token(4), [{"artifact": "a.log", "revision": "r"}]]),
        ),
        at(10.0, "parent", "verify_task", json!([1, token(4), "r"])),
        at(11.0, "parent", "revoke", json!([1, "too late"])),
    ]));
}

/// `test_revoke_skips_the_message_when_the_previous_owner_inbox_is_full`,
/// with the hundred unread messages written directly (S11 ports `send`);
/// one read message leaves room again.
#[test]
fn revoke_skips_the_message_when_the_previous_owner_inbox_is_full() {
    let fill = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<100)
         INSERT INTO messages(sender,recipient,body,status) SELECT 'parent','worker','noise','accepted' FROM n;";
    run_golden(&claimed([
        sql(fill),
        at(5.0, "parent", "revoke", json!([1, "silent member"])),
        at(6.0, "worker", "claim", json!([1])),
        sql("UPDATE messages SET status='consumed' WHERE id=1"),
        at(7.0, "parent", "revoke", json!([1, "still silent"])),
    ]));
}

/// `test_revoke_after_confirmed_death_records_the_audit_without_a_message`.
#[test]
fn revoke_after_death_records_the_audit_without_a_message() {
    run_golden(&claimed([
        at(5.0, "worker", "reserve", json!([1, token(3), ["src/a.rs"]])),
        dead("worker"),
        at(6.0, "parent", "revoke", json!([1, "reassign after death"])),
        owners(7.0),
    ]));
}

/// A previous owner's row holding what the board never writes (#2321
/// review): `revoke` and `recover` read only its status, as Python's
/// `SELECT status FROM members WHERE id=?` does, so text that is not UTF-8
/// in another column stops neither; a status that is not UTF-8 is the
/// same refusal on both; bytes reading `dead` are not `dead` to `recover`.
#[test]
fn revoke_and_recover_read_only_the_owners_status() {
    let corrupt = |column: &str| {
        sql(&format!(
            "UPDATE members SET {column}=CAST(X'FF' AS TEXT) WHERE id='worker'"
        ))
    };
    run_golden(&claimed([
        corrupt("socket"),
        at(5.0, "parent", "revoke", json!([1, "x"])),
        raw(6.0),
    ]));
    run_golden(&claimed([
        corrupt("socket"),
        dead("worker"),
        at(5.0, "parent", "recover", json!([1])),
        raw(6.0),
    ]));
    run_golden(&claimed([
        corrupt("status"),
        at(5.0, "parent", "revoke", json!([1, "x"])),
        at(6.0, "parent", "recover", json!([1])),
    ]));
    run_golden(&claimed([
        sql("UPDATE members SET status=CAST('dead' AS BLOB) WHERE id='worker'"),
        at(5.0, "parent", "recover", json!([1])),
    ]));
}

/// `test_stale_claim_cannot_modify_recovered_work`, in full: recovery
/// needs the owner's confirmed death, and the old token is stale after it.
#[test]
fn stale_claim_cannot_modify_recovered_work() {
    run_golden(&claimed([
        at(5.0, "parent", "recover", json!([1])),
        dead("worker"),
        at(6.0, "parent", "recover", json!([1])),
        at(7.0, "parent", "claim", json!([1])),
        at(8.0, "worker", "release", json!([1, token(3)])),
        at(9.0, "parent", "release", json!([1, token(3)])),
        raw(10.0),
    ]));
}

/// Retained reservations are released only by `release_files` exactly
/// `True` (Python's `is True`: `1` and `"true"` are not).
#[test]
fn recover_releases_retained_reservations_only_when_told_true() {
    run_golden(&claimed([
        at(
            5.0,
            "worker",
            "reserve",
            json!([1, token(3), ["src/a.rs", "b.rs"]]),
        ),
        dead("worker"),
        at(6.0, "parent", "recover", json!([1])),
        at(6.1, "parent", "recover", json!([1, 1])),
        at(6.2, "parent", "recover", json!([1, "true"])),
        at(6.3, "worker", "recover", json!([1, true])),
        at(
            7.0,
            "parent",
            "recover",
            json!({"task_id": 1, "release_files": true}),
        ),
        owners(8.0),
        raw(9.0),
        at(10.0, "parent", "recover", json!([1, true])),
    ]));
}

/// A run with the criteria `tests` (command) and `review` (review).
fn two_criteria() -> Step {
    step(
        "parent",
        "create_run",
        json!({
            "goal": "ship feature",
            "constraints": ["clean architecture"],
            "criteria": [
                {"id": "tests", "kind": "command", "description": "acceptance tests pass"},
                {"id": "review", "kind": "review", "description": "independent review"},
            ],
            "member_limit": 3,
            "deadline": NOW + 300.0,
        }),
        NOW,
    )
}

/// `test_empty_queue_and_submissions_do_not_prove_success`.
#[test]
fn empty_queue_and_submissions_do_not_prove_success() {
    let mut steps = vec![two_criteria()];
    steps.extend(joined(1.0, "worker"));
    steps.extend([
        at(2.0, "parent", "complete", json!(["abc"])),
        at(
            3.0,
            "worker",
            "task_create",
            json!(["task", "work", ["tests pass"]]),
        ),
        at(4.0, "worker", "claim", json!([1])),
        at(
            5.0,
            "worker",
            "submit",
            json!([1, token(3), [{"artifact": "tests.log", "revision": "abc"}]]),
        ),
        at(
            6.0,
            "parent",
            "evidence",
            json!(["tests", "tests.log", "abc", "command", true]),
        ),
        at(
            7.0,
            "parent",
            "evidence",
            json!(["review", "review.md", "abc", "review", true]),
        ),
        at(8.0, "parent", "complete", json!(["abc"])),
        at(9.0, "parent", "verify_task", json!([1, token(3), "abc"])),
        at(10.0, "parent", "complete", json!(["changed"])),
        at(11.0, "parent", "complete", json!(["abc"])),
        at(12.0, "parent", "_snapshot", json!([])),
    ]);
    run_golden(&steps);
}

/// `test_workers_cannot_accept_overall_completion`: a worker's pass is a
/// proposal, never acceptance.
#[test]
fn workers_cannot_accept_overall_completion() {
    let mut steps = vec![two_criteria()];
    steps.extend(joined(1.0, "worker"));
    steps.extend([
        at(2.0, "worker", "complete", json!(["abc"])),
        at(
            3.0,
            "worker",
            "evidence",
            json!(["tests", "tests.log", "abc", "command", true]),
        ),
        at(
            4.0,
            "worker",
            "evidence",
            json!(["tests", "tests.log", "abc", "command", true]),
        ),
    ]);
    run_golden(&steps);
}

/// P3: the arguments Python type-checks at run time, and those it binds
/// untyped, are refused or stored as Python refuses or stores them.
#[test]
fn reservation_arguments_are_checked_as_python_checks_them() {
    let many: Vec<Value> = (0..101).map(|n| json!(format!("f{n}"))).collect();
    run_golden(&claimed([
        at(5.0, "worker", "reserve", json!([1, token(3), "a.rs"])),
        at(5.1, "worker", "reserve", json!([1, token(3), []])),
        at(5.2, "worker", "reserve", json!([1, token(3), many])),
        at(5.3, "worker", "reserve", json!([1, token(3), ["a.rs", 5]])),
        at(5.4, "worker", "reserve", json!([1, token(3), [" "]])),
        at(
            5.5,
            "worker",
            "reserve",
            json!([1, token(3), ["x".repeat(4097)]]),
        ),
        at(
            5.6,
            "worker",
            "reserve",
            json!([1, token(3), ["x".repeat(4096)]]),
        ),
        at(5.7, "worker", "reserve", json!(["1", token(3), ["s.rs"]])),
        at(5.8, "worker", "reserve", json!([true, token(3), ["t.rs"]])),
        at(5.9, "worker", "reserve", json!([1, 3, ["u.rs"]])),
        at(6.0, "worker", "release_files", json!([1, token(3), [1]])),
        at(
            6.1,
            "worker",
            "release_files",
            json!([1.0, token(3), token(4)]),
        ),
        at(7.0, "parent", "file_owners", json!([-1])),
        at(7.1, "parent", "file_owners", json!([true])),
        at(7.2, "parent", "file_owners", json!([0.0])),
        at(7.3, "parent", "file_owners", json!(["0"])),
        at(7.4, "parent", "file_owners", json!([0, 0])),
        at(7.5, "parent", "file_owners", json!([0, 101])),
        at(
            7.6,
            "parent",
            "file_owners",
            json!({"limit": 2, "offset": 1}),
        ),
        at(7.7, "parent", "file_owners", json!([0, 100])),
        at(8.0, "parent", "revoke", json!([1, 5])),
        at(8.1, "parent", "revoke", json!([1, "x".repeat(8193)])),
        at(8.2, "parent", "revoke", json!([true, "boolean id"])),
        at(8.3, "worker", "claim", json!([1])),
        at(8.4, "parent", "revoke", json!([1.0, "float id"])),
        at(8.5, "worker", "claim", json!([1])),
        at(8.6, "parent", "revoke", json!(["1", "text id"])),
        at(8.7, "parent", "revoke", json!([9, "unknown"])),
        at(8.8, "parent", "recover", json!([9])),
    ]));
}

/// Reserving, recovering and revoking need a running run
/// (`authorize(active=True)`, the #2316 paused-run table's sibling; the
/// #2321 mutation report's survivors): on a paused run each is refused
/// with Python's own text, and the board is left as it was. Each op is one
/// the unpaused board grants, so only the run's status refuses it: the
/// worker reserves under its claim of task 1, and the parent recovers it
/// from the dead worker or revokes it from the live one. Releasing and
/// paging reservations need no running run (`active=False` and a read), so
/// the paused board grants both.
#[test]
fn reserving_recovering_and_revoking_are_refused_on_a_paused_run() {
    let reserved = || at(5.0, "worker", "reserve", json!([1, token(3), ["a.rs"]]));
    let ops = [
        (
            claimed([]),
            at(9.0, "worker", "reserve", json!([1, token(3), ["b.rs"]])),
        ),
        (
            claimed([reserved(), dead("worker")]),
            at(9.0, "parent", "recover", json!([1, true])),
        ),
        (
            claimed([reserved()]),
            at(9.0, "parent", "revoke", json!([1, "reassign"])),
        ),
    ];
    let paused = sql("UPDATE run SET status='paused'");
    let refusal = "run is paused; no new work permitted";
    for (setup, op) in ops {
        let method = op.method.clone();
        let granted = [setup.clone(), vec![op.clone()]].concat();
        assert!(
            matches!(run_rust(&granted), Outcome::Ok(_)),
            "{method} is granted on the running run"
        );
        let steps = [setup, vec![paused.clone(), op, owners(10.0), raw(11.0)]].concat();
        run_golden(&steps);
        let outcome = run_rust(&steps[..steps.len() - 2]);
        assert!(
            matches!(&outcome, Outcome::Refused(text) if text == refusal),
            "{method}: {outcome:?}"
        );
    }
    run_golden(&claimed([
        reserved(),
        paused,
        owners(9.0),
        at(
            10.0,
            "worker",
            "release_files",
            json!([1, token(3), token(4)]),
        ),
        owners(11.0),
    ]));
}

/// The revocation message names the task id as Python's `str()` writes it
/// (#2321 mutation report): an integer in decimal, and a float that finds
/// the task as `repr` writes it, `1.0`, or `1e+16` for the task a hand
/// edit numbered 10**16.
#[test]
fn the_revocation_message_names_the_task_id_as_python_writes_it() {
    run_golden(&claimed([
        at(5.0, "parent", "revoke", json!([1.0, "a float"])),
        at(6.0, "worker", "claim", json!([1])),
        at(7.0, "parent", "revoke", json!([1, "an integer"])),
        at(8.0, "worker", "claim", json!([1])),
        sql("UPDATE tasks SET id=10000000000000000 WHERE id=1"),
        at(9.0, "parent", "revoke", json!([1e16, "a large float"])),
        at(10.0, "worker", "claim", json!([1e16])),
        at(
            11.0,
            "parent",
            "revoke",
            json!([10_000_000_000_000_000_i64, "its integer"]),
        ),
    ]));
}

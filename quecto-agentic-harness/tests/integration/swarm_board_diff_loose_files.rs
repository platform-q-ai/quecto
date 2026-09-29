//! The divergences of file reservations, recovery and revocation (#2275;
//! epic #2265 P3), named in
//! `swarm_board_diff_loose::PERMITTED_DIVERGENCES` and pinned here by the
//! test of the same name (or, for a name pinned in `src`, listed in
//! [`SLICE_PINS`]):
//!
//! - `multi_conflict_names_the_smallest_path` (P3-a): a `reserve` meeting
//!   several reserved paths names the smallest in sort order; Python names
//!   whichever its hash-seeded `set` meets first, which varies run to run,
//!   so no differential scenario holds more than one conflict. The rows
//!   inserted are the same set (Rust inserts them sorted, and the
//!   comparator orders `files` by path). Also pinned by
//!   `reserve_files_tests`.
//! - `non_utf8_resolved_path_is_refused` (PR #2321 review): a reserved
//!   path that resolves, through a symlink a worker made in the shared
//!   checkout, to a name that is not UTF-8 is refused as an escape. Python's
//!   `resolve()` answers it with a surrogate escape, which its `sqlite3`
//!   cannot encode, so the call raises `UnicodeEncodeError` (not a
//!   `SwarmError`). Also pinned by `checkout_paths_tests`.
//! - `integer_beyond_i64_is_refused`, as the pin table describes it, for
//!   `file_owners`' offset too: Python binds `(limit, offset)`, so the
//!   Rust refusal names parameter 2.
//! - `unknown_member_status_is_not_alive`, as the pin table describes it,
//!   for `revoke` too: only a live or reserved previous owner is told,
//!   where Python messages any owner not `dead` (a NULL, unknown or BLOB
//!   status). Pinned by `board_recovery_tests` (the status alone is read,
//!   and bytes read as no status: `contracts::board_members`).
use serde_json::json;

use crate::swarm_board_diff_files::{claimed, token};
use crate::swarm_board_diff_loose::PERMITTED_DIVERGENCES;
use crate::swarm_board_diff_membership::at;
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_rust, step, symlink_bytes, try_run_golden,
};

/// This slice's divergences pinned in `src`: the name, the test file's
/// source and the pinning test in it.
const SLICE_PINS: [(&str, &str, &str); 3] = [
    (
        "multi_conflict_names_the_smallest_path",
        include_str!("../../src/application/swarm/use_cases/reserve_files_tests.rs"),
        "multi_conflict_names_the_smallest_path",
    ),
    (
        "non_utf8_resolved_path_is_refused",
        include_str!("../../src/infrastructure/workspace/checkout_paths_tests.rs"),
        "a_path_resolving_to_a_name_that_is_not_utf8_is_refused",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/board_recovery_tests.rs"),
        "an_unknown_owner_status_is_not_told",
    ),
];

const ESCAPE: &str = "file must resolve inside the shared checkout";

#[test]
fn this_slices_pins_in_src_exist() {
    for (name, source, test) in SLICE_PINS {
        assert!(PERMITTED_DIVERGENCES.contains(&name), "{name}");
        assert!(source.contains(&format!("fn {test}()")), "{test}");
    }
}

/// Task 1 claimed by `worker` under [`token`]`(3)`, and task 2 by the
/// parent under `token(4)`.
fn both_claimed() -> Vec<Step> {
    claimed([
        at(
            5.0,
            "parent",
            "task_create",
            json!(["second", "other", ["pass"]]),
        ),
        at(6.0, "parent", "claim", json!([2])),
    ])
}

/// Several reserved paths met: the Rust board names the smallest, not the
/// first given. (Python's choice varies run to run, so no side compares.)
#[test]
fn multi_conflict_names_the_smallest_path() {
    let mut steps = both_claimed();
    steps.extend([
        at(
            7.0,
            "worker",
            "reserve",
            json!([1, token(3), ["m.rs", "b.rs", "y.rs"]]),
        ),
        at(
            8.0,
            "parent",
            "reserve",
            json!([2, token(4), ["z.rs", "y.rs", "free.rs", "b.rs"]]),
        ),
    ]);
    assert_eq!(
        run_rust(&steps),
        Outcome::Refused(
            "file already reserved: b.rs; acquire the entire set or release and retry".to_owned()
        )
    );
}

/// A symlink to the byte `0xff`: Python raises, the Rust board refuses.
#[test]
fn non_utf8_resolved_path_is_refused() {
    let steps = claimed([
        symlink_bytes("l", b"\xff"),
        at(5.0, "worker", "reserve", json!([1, token(3), ["l/x"]])),
    ]);
    let difference = try_run_golden(&steps, |_, _, _| {}).unwrap_err();
    assert!(
        difference.contains(": reserve as worker ")
            && difference.contains("Python raised UnicodeEncodeError"),
        "{difference}"
    );
    assert_eq!(run_rust(&steps), Outcome::Refused(ESCAPE.to_owned()));
}

/// `file_owners` with an offset beyond i64 but within u64.
#[test]
fn integer_beyond_i64_is_refused() {
    let args = json!([u64::MAX]);
    let steps = [
        step("parent", "bootstrap_run", json!([7, "s", null]), NOW),
        step("parent", "file_owners", args.clone(), NOW + 1.0),
    ];
    let refused = Outcome::Refused(
        "coordination store unavailable or contended: Error binding parameter 2: \
         Python int too large to convert to SQLite INTEGER"
            .to_owned(),
    );
    let difference = try_run_golden(&steps, |_, _, _| {}).unwrap_err();
    assert_eq!(
        difference,
        format!(
            "step 1: file_owners as parent with {args} at {}: Python raised \
             OverflowError: Python int too large to convert to SQLite INTEGER\n  \
             rust   {refused:?}",
            NOW + 1.0
        )
    );
    assert_eq!(run_rust(&steps), refused);
}

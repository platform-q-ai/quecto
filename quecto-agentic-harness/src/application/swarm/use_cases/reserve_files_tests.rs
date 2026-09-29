use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::ReserveFiles;
use crate::application::swarm::board_test_support::{
    BoardState, CounterIds, LexicalCheckout, MemoryBoard, SteppingClock, StoredFile, member_row,
    running_board, stored_task,
};
use crate::application::swarm::dto::{Reservation, ReserveFilesRequest};
use crate::domain::swarm::{BoardError, RefusalKind};

fn file(path: &str, task: i64) -> StoredFile {
    StoredFile {
        path: path.to_owned(),
        task,
        owner: "parent".to_owned(),
        claim: "stored-token".to_owned(),
        token: "held".to_owned(),
    }
}

/// Task 1 claimed by the worker (token `stored-token`), task 2 by the
/// parent, which holds `taken.rs` and `z.rs`.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        stored_task(2, "claimed", json!([]), Some("parent")),
    ];
    state.files = vec![file("taken.rs", 2), file("z.rs", 2)];
    state
}

fn service(board: &Arc<MemoryBoard>) -> ReserveFiles {
    let aliases = BTreeMap::from([("alias".to_owned(), "real".to_owned())]);
    ReserveFiles::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        CounterIds::journalling(&board.journal),
        Arc::new(LexicalCheckout { aliases }),
    )
}

fn reserve(task_id: Value, token: &str, paths: Value) -> ReserveFilesRequest {
    ReserveFilesRequest {
        actor: "worker".to_owned(),
        task_id,
        token: json!(token),
        paths,
    }
}

/// The owner reserves the normalised, distinct paths in sorted order under
/// one drawn ownership token; the event names them sorted and the task id
/// as given.
#[test]
fn the_owner_reserves_normalised_paths_in_sorted_order() {
    let board = MemoryBoard::with(board());
    let reserved = service(&board)
        .execute(reserve(
            json!("1"),
            "stored-token",
            json!(["src/../b.rs", "./a.rs", "alias/new", "a.rs"]),
        ))
        .unwrap();
    let paths = ["a.rs", "b.rs", "real/new"];
    assert_eq!(
        reserved,
        Reservation {
            task_id: json!(1),
            token: format!("{:032x}", 1),
            paths: paths.map(str::to_owned).to_vec(),
        }
    );
    let state = board.snapshot();
    let inserted: Vec<&StoredFile> = state.files.iter().filter(|f| f.task == 1).collect();
    assert_eq!(
        inserted.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        paths
    );
    for file in inserted {
        assert_eq!(
            (
                file.owner.as_str(),
                file.claim.as_str(),
                file.token.as_str()
            ),
            ("worker", "stored-token", reserved.token.as_str())
        );
    }
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        (
            "files_reserved",
            &json!({"task": "1", "paths": ["a.rs", "b.rs", "real/new"]})
        )
    );
}

/// The paths are checked before the gate, in Python's order: the list's
/// size, then each path's bound and its normalisation.
#[test]
fn paths_are_checked_before_the_gate() {
    let board = MemoryBoard::with(BoardState::default());
    let service = service(&board);
    let many: Vec<Value> = (0..101).map(|n| json!(format!("f{n}"))).collect();
    for (paths, message) in [
        (json!([]), "reserve 1 through 100 paths together"),
        (json!("a.rs"), "reserve 1 through 100 paths together"),
        (Value::Array(many), "reserve 1 through 100 paths together"),
        (
            json!(["a.rs", 5]),
            "path must be nonempty and at most 4096 bytes",
        ),
        (json!([" "]), "path must be nonempty and at most 4096 bytes"),
        (
            json!(["x".repeat(4097)]),
            "path must be nonempty and at most 4096 bytes",
        ),
        (
            json!(["../escape", 5]),
            "file must resolve inside the shared checkout",
        ),
        (
            json!(["/abs"]),
            "file must resolve inside the shared checkout",
        ),
    ] {
        assert_eq!(
            service
                .execute(reserve(json!(1), "stored-token", paths.clone()))
                .unwrap_err(),
            BoardError::new(RefusalKind::Invalid, message),
            "{paths}"
        );
    }
    assert!(board.transactions().is_empty(), "no gate ran");
}

/// A stale token, a reservation already held, or a full board refuses the
/// whole set: nothing is inserted, no token drawn, no event recorded.
#[test]
fn a_conflict_or_a_full_board_reserves_nothing() {
    let mut full = board();
    full.files
        .extend((0..997).map(|n| file(&format!("f{n:04}"), 2)));
    for (state, token, paths, (kind, message)) in [
        (
            board(),
            "stale",
            json!(["a.rs"]),
            (RefusalKind::StaleToken, "stale or unowned claim"),
        ),
        (
            board(),
            "stored-token",
            json!(["a.rs", "taken.rs"]),
            (
                RefusalKind::ReservedByOther,
                "file already reserved: taken.rs; acquire the entire set or release and retry",
            ),
        ),
        (
            full,
            "stored-token",
            json!(["a.rs", "b.rs"]),
            (
                RefusalKind::CapacityFull,
                "file reservation board full (1000); release settled work",
            ),
        ),
    ] {
        let before = state.files.clone();
        let board = MemoryBoard::with(state);
        assert_eq!(
            service(&board)
                .execute(reserve(json!(1), token, paths))
                .unwrap_err(),
            BoardError::new(kind, message)
        );
        let after = board.snapshot();
        assert_eq!(after.files, before);
        assert!(after.events.is_empty());
        assert!(
            !board
                .journal()
                .iter()
                .any(|entry| entry.starts_with("draw"))
        );
    }
}

/// Exactly a thousand reservations fit.
#[test]
fn the_thousandth_reservation_fits() {
    let mut state = board();
    state
        .files
        .extend((0..997).map(|n| file(&format!("f{n:04}"), 2)));
    let board = MemoryBoard::with(state);
    service(&board)
        .execute(reserve(json!(1), "stored-token", json!(["a.rs"])))
        .unwrap();
    assert_eq!(board.snapshot().files.len(), 1000);
}

/// `multi_conflict_names_the_smallest_path` (P3-a, #2275): with several
/// paths already reserved, the refusal names the smallest in sort order.
/// Python names whichever its hash-seeded `set` meets first, so no
/// differential scenario may hold more than one conflict.
#[test]
fn multi_conflict_names_the_smallest_path() {
    let board = MemoryBoard::with(board());
    assert_eq!(
        service(&board)
            .execute(reserve(
                json!(1),
                "stored-token",
                json!(["z.rs", "a.rs", "taken.rs"]),
            ))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::ReservedByOther,
            "file already reserved: taken.rs; acquire the entire set or release and retry"
        )
    );
}

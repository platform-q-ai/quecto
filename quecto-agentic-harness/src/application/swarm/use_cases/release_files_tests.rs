use serde_json::{Value, json};

use super::ReleaseFiles;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::ReleaseFilesRequest;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

fn file(path: &str, owner: &str, claim: &str, token: &str) -> StoredFile {
    StoredFile {
        path: path.to_owned(),
        task: 1,
        owner: owner.to_owned(),
        claim: claim.to_owned(),
        token: token.to_owned(),
    }
}

/// Task 1 claimed by the worker under `stored-token`, with two
/// reservation sets of that claim and one of an older claim.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![stored_task(1, "claimed", json!([]), Some("worker"))];
    state.files = vec![
        file("a", "worker", "stored-token", "r1"),
        file("b", "worker", "stored-token", "r2"),
        file("c", "worker", "older", "r1"),
    ];
    state
}

fn release(token: &str, reservation: Value) -> ReleaseFilesRequest {
    ReleaseFilesRequest {
        actor: "worker".to_owned(),
        task_id: json!(1),
        token: json!(token),
        reservation,
    }
}

/// Only the owner's current claim releases, and only the named set goes;
/// the event names the reservation as given, and an unknown reservation
/// is still recorded, as Python records it.
#[test]
fn the_owner_releases_one_reservation_set() {
    let board = MemoryBoard::with(board());
    let service = ReleaseFiles::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(release("stale", json!("r1"))).unwrap_err(),
        BoardError::new(RefusalKind::StaleToken, "stale or unowned claim")
    );
    service
        .execute(release("stored-token", json!("r1")))
        .unwrap();
    service
        .execute(release("stored-token", json!("none")))
        .unwrap();
    let state = board.snapshot();
    let paths: Vec<&str> = state.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["b", "c"]);
    let details: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|e| (e.action.as_str(), &e.detail))
        .collect();
    assert_eq!(
        details,
        [
            ("files_released", &json!({"task": 1, "token": "r1"})),
            ("files_released", &json!({"task": 1, "token": "none"})),
        ]
    );
}

/// `operation(active=False)`: a paused run still lets the owner release.
#[test]
fn a_paused_run_still_releases() {
    let mut state = board();
    state.run.as_mut().unwrap().record.status = Some(RunState::new("paused"));
    let board = MemoryBoard::with(state);
    ReleaseFiles::new(board.clone(), SteppingClock::fixed(50.0))
        .execute(release("stored-token", json!("r2")))
        .unwrap();
    assert_eq!(board.snapshot().files.len(), 2);
}

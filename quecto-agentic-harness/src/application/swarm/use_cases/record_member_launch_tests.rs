use serde_json::{Value, json};

use super::RecordMemberLaunch;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board, stored_member,
};
use crate::application::swarm::dto::{LaunchIdentity, RecordMemberLaunchRequest};
use crate::domain::swarm::BoardError;

fn record(member: &str, reservation: &str, pid: i64, started: &str) -> RecordMemberLaunchRequest {
    RecordMemberLaunchRequest {
        actor: "parent".to_owned(),
        member: member.to_owned(),
        reservation: reservation.to_owned(),
        launch: LaunchIdentity {
            pid,
            started: started.to_owned(),
        },
    }
}

fn with_worker(status: &str, pid: Value, started: Value) -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(stored_member(
        "worker",
        json!("worker-reservation"),
        status,
        [pid, started, json!(null), json!("parent")],
    ));
    state
}

#[test]
fn a_launch_is_recorded_once_and_its_retry_is_accepted() {
    let board = MemoryBoard::with(with_worker("reserved", json!(null), json!(null)));
    let service = RecordMemberLaunch::new(board.clone(), SteppingClock::fixed(50.0));
    service
        .execute(record("worker", "worker-reservation", 7, "Mon"))
        .unwrap();
    service
        .execute(record("worker", "worker-reservation", 7, "Mon"))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(
        state.members[1],
        stored_member(
            "worker",
            json!("worker-reservation"),
            "reserved",
            [json!(7), json!("Mon"), json!(null), json!("parent")]
        ),
        "the status is left as it is"
    );
    assert!(state.events.is_empty(), "a launch record is no event");
}

#[test]
fn a_stale_reservation_or_another_process_is_refused() {
    let service =
        |state| RecordMemberLaunch::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
    let stale = BoardError::new("stale launch reservation");
    for (state, member, reservation) in [
        (
            with_worker("reserved", json!(null), json!(null)),
            "worker",
            "other",
        ),
        (
            with_worker("reserved", json!(null), json!(null)),
            "stranger",
            "worker-reservation",
        ),
        (
            with_worker("dead", json!(null), json!(null)),
            "worker",
            "worker-reservation",
        ),
    ] {
        assert_eq!(
            service(state)
                .execute(record(member, reservation, 7, "Mon"))
                .unwrap_err(),
            stale,
            "{member} {reservation}"
        );
    }
    for (pid, started) in [(8, "Mon"), (7, "Tue")] {
        assert_eq!(
            service(with_worker("live", json!(7), json!("Mon")))
                .execute(record("worker", "worker-reservation", pid, started))
                .unwrap_err(),
            BoardError::new("conflicting launch identity")
        );
    }
    // A pid stored as the equal float is the same process.
    service(with_worker("reserved", json!(7.0), json!("Mon")))
        .execute(record("worker", "worker-reservation", 7, "Mon"))
        .unwrap();
}

/// The owner-decided divergence `unknown_member_status_is_not_alive`
/// (#2295, listed in `tests/integration/swarm_board_diff_loose.rs`):
/// Python records a launch for any status but `'dead'`; Rust only for a
/// `live` or `reserved` member.
#[test]
fn an_unknown_member_status_records_no_launch() {
    for status in [json!(null), json!("weird")] {
        let mut state = running_board(100.0);
        let mut row = member_row("worker", "reserved");
        row.columns[2].1 = status.clone();
        state.members.push(row);
        let service = RecordMemberLaunch::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
        assert_eq!(
            service
                .execute(record("worker", "worker-reservation", 7, "Mon"))
                .unwrap_err(),
            BoardError::new("stale launch reservation"),
            "{status}"
        );
    }
}

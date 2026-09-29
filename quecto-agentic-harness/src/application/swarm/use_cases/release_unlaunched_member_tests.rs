use serde_json::json;

use super::ReleaseUnlaunchedMember;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board, stored_member,
};
use crate::application::swarm::dto::ReleaseUnlaunchedMemberRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn release(member: &str) -> ReleaseUnlaunchedMemberRequest {
    ReleaseUnlaunchedMemberRequest {
        actor: "parent".to_owned(),
        member: json!(member),
    }
}

#[test]
fn only_a_reserved_member_without_a_pid_is_released() {
    let mut state = running_board(100.0);
    state.members.push(member_row("unlaunched", "reserved"));
    state.members.push(stored_member(
        "launched",
        json!("r"),
        "reserved",
        [json!(7), json!("Mon"), json!(null), json!("parent")],
    ));
    state.members.push(member_row("odd", "weird"));
    let board = MemoryBoard::with(state);
    let service = ReleaseUnlaunchedMember::new(board.clone(), SteppingClock::fixed(50.0));
    service.execute(release("unlaunched")).unwrap();
    let refused = BoardError::new(
        RefusalKind::WrongState,
        "only an unlaunched reservation may be released",
    );
    for member in ["unlaunched", "launched", "parent", "odd", "stranger"] {
        assert_eq!(
            service.execute(release(member)).unwrap_err(),
            refused,
            "{member}"
        );
    }
    let state = board.snapshot();
    assert_eq!(state.members[1].text("status"), Some("dead"));
    assert_eq!(state.events.len(), 1);
    let event = &state.events[0];
    assert_eq!(
        (event.actor.as_str(), event.action.as_str(), &event.detail),
        (
            "parent",
            "launch_abandoned",
            &json!({"member": "unlaunched"})
        )
    );
}

/// Served over another repository (#2303 reconcile), the release writes to
/// that board alone.
#[test]
fn over_releases_on_the_given_board() {
    let mut state = running_board(100.0);
    state.members.push(member_row("unlaunched", "reserved"));
    let composed_over = MemoryBoard::with(running_board(100.0));
    let other = MemoryBoard::with(state);
    ReleaseUnlaunchedMember::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(release("unlaunched"))
        .unwrap();
    assert_eq!(other.snapshot().members[1].text("status"), Some("dead"));
    assert!(composed_over.transactions().is_empty());
}

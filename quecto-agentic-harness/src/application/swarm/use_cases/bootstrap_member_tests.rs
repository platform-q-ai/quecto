use serde_json::{Value, json};

use super::BootstrapMember;
use crate::application::swarm::board_test_support::{CounterIds, MemoryBoard, SteppingClock};
use crate::application::swarm::dto::{BootstrapMemberRequest, Joined, RunSummary};
use crate::application::swarm::use_cases::OverRepository;

fn bootstrap() -> BootstrapMemberRequest {
    BootstrapMemberRequest {
        member: "parent".to_owned(),
        pid: json!(1),
        started: json!("s"),
        socket: Value::Null,
        reservation: Value::Null,
    }
}

fn service(board: &std::sync::Arc<MemoryBoard>) -> BootstrapMember {
    BootstrapMember::new(
        board.clone(),
        SteppingClock::fixed(1.0),
        CounterIds::journalling(&board.journal),
    )
}

/// The placeholder in a creating transaction, then the join (the member's
/// own live row), then the coordinator's summary of the setup run; a
/// second call finds the run.
#[test]
fn the_placeholder_then_the_join_then_the_summary() {
    let board = MemoryBoard::default().into();
    let first = service(&board).execute(bootstrap()).unwrap();
    assert!(first.created);
    assert_eq!(
        first.joined,
        Joined::AlreadyLive {
            coordinator: Some("parent".to_owned())
        }
    );
    let RunSummary::Full(summary) = first.summary else {
        panic!("the full summary");
    };
    assert_eq!(summary.run.get("status"), Some(&json!("setup")));
    assert_eq!(board.transactions().first(), Some(&true), "creating first");
    assert!(!service(&board).execute(bootstrap()).unwrap().created);
}

#[test]
fn over_bootstraps_on_the_given_board() {
    let composed_over: std::sync::Arc<MemoryBoard> = MemoryBoard::default().into();
    let other: std::sync::Arc<MemoryBoard> = MemoryBoard::default().into();
    service(&composed_over)
        .over(other.clone())
        .execute(bootstrap())
        .unwrap();
    assert!(composed_over.transactions().is_empty());
    assert!(other.snapshot().run.is_some());
}

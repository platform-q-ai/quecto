use serde_json::json;

use super::RegisterMemberSocket;
use crate::application::swarm::board_test_support::{MemoryBoard, SteppingClock, running_board};
use crate::application::swarm::dto::RegisterMemberSocketRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn register(actor: &str, socket: Option<&str>) -> RegisterMemberSocketRequest {
    RegisterMemberSocketRequest {
        actor: actor.to_owned(),
        socket: socket.map_or(serde_json::Value::Null, serde_json::Value::from),
    }
}

#[test]
fn the_actor_registers_its_own_endpoint() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = RegisterMemberSocket::new(board.clone(), SteppingClock::fixed(50.0));
    service
        .execute(register("parent", Some("/p.sock")))
        .unwrap();
    assert_eq!(
        board.snapshot().members[0].get("socket"),
        Some(&json!("/p.sock"))
    );
    service.execute(register("parent", None)).unwrap();
    assert_eq!(
        board.snapshot().members[0].get("socket"),
        Some(&json!(null))
    );
    assert!(board.snapshot().events.is_empty());
    assert_eq!(
        service
            .execute(register("stranger", Some("/s")))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
}

/// Served over another repository (#2303 reconcile), the endpoint is
/// registered on that board alone.
#[test]
fn over_registers_on_the_given_board() {
    let composed_over = MemoryBoard::with(running_board(100.0));
    let other = MemoryBoard::with(running_board(100.0));
    RegisterMemberSocket::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(register("parent", Some("/p.sock")))
        .unwrap();
    assert_eq!(
        other.snapshot().members[0].get("socket"),
        Some(&json!("/p.sock"))
    );
    assert!(composed_over.transactions().is_empty());
}

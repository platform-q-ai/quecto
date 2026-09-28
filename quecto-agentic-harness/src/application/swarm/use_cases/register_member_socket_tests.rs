use serde_json::json;

use super::RegisterMemberSocket;
use crate::application::swarm::board_test_support::{MemoryBoard, SteppingClock, running_board};
use crate::application::swarm::dto::RegisterMemberSocketRequest;
use crate::domain::swarm::BoardError;

fn register(actor: &str, socket: Option<&str>) -> RegisterMemberSocketRequest {
    RegisterMemberSocketRequest {
        actor: actor.to_owned(),
        socket: socket.map(str::to_owned),
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
        BoardError::new("invoking member is unknown or death confirmed")
    );
}

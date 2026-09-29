use super::ReadControlStatus;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board,
};
use crate::domain::swarm::BoardError;

/// A dead member still reads the receipt; an unknown one is refused; the
/// read writes nothing.
#[test]
fn the_receipt_is_read_by_any_member_even_a_dead_one() {
    let mut state = paused(running_board(100.0), 40.0, Some(("blocked", "why")));
    state.members.push(member_row("gone", "dead"));
    let board = MemoryBoard::with(state);
    let service = ReadControlStatus::new(board.clone(), SteppingClock::fixed(50.0));
    let receipt = service.execute("gone").unwrap();
    assert_eq!(
        (receipt.status.as_deref(), receipt.outcome.as_deref()),
        (Some("paused"), Some("blocked"))
    );
    assert_eq!(
        service.execute("stranger").unwrap_err(),
        BoardError::new("invoking member is unknown or death confirmed")
    );
    assert!(board.journal().is_empty(), "{:?}", board.journal());
}

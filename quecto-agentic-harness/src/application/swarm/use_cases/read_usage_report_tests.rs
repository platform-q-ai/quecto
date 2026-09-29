use serde_json::json;

use super::ReadUsageReport;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board, usage,
};
use crate::domain::swarm::BoardError;

/// Any member, a dead one included, reads the report the board holds.
#[test]
fn the_report_is_read_through_the_gate() {
    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    let held = usage(json!({"token_limit": 5}), 3, 1);
    state.usage = Some(held.clone());
    let board = MemoryBoard::with(state);
    let service = ReadUsageReport::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(service.execute("gone").unwrap(), held);
    assert_eq!(
        service.execute("stranger").unwrap_err(),
        BoardError::new("invoking member is unknown or death confirmed")
    );
}

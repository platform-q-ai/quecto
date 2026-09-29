use super::ReadEventCursor;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, RecordedEvent, running_board,
};
use crate::application::swarm::use_cases::OverRepository;

fn event(action: &str) -> RecordedEvent {
    RecordedEvent {
        actor: "parent".to_owned(),
        time: 1.0,
        action: action.to_owned(),
        detail: serde_json::json!({}),
    }
}

#[test]
fn a_board_without_events_is_at_cursor_zero() {
    let board = MemoryBoard::with(BoardState::default());
    assert_eq!(ReadEventCursor::new(board.clone()).execute().unwrap(), 0);
    assert_eq!(
        board.transactions(),
        [false],
        "one plain transaction that never creates a board"
    );
}

#[test]
fn the_cursor_is_the_latest_event_id_and_reading_it_writes_nothing() {
    let mut state = running_board(250.0);
    state.events = vec![event("created"), event("claimed"), event("sent")];
    let board = MemoryBoard::with(state);
    let cursor = ReadEventCursor::new(board.clone());
    assert_eq!(cursor.execute().unwrap(), 3);
    assert_eq!(cursor.execute().unwrap(), 3, "a read never moves it");
    assert_eq!(board.snapshot().events.len(), 3, "a read writes nothing");
}

#[test]
fn over_another_repository_reads_that_repository() {
    let empty = MemoryBoard::with(BoardState::default());
    let mut state = running_board(250.0);
    state.events = vec![event("created")];
    let other = MemoryBoard::with(state);
    let cursor = ReadEventCursor::new(empty.clone()).over(other.clone());
    assert_eq!(cursor.execute().unwrap(), 1);
    assert!(
        empty.transactions().is_empty(),
        "the composed one is untouched"
    );
}

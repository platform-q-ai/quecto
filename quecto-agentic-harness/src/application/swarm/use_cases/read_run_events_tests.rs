use serde_json::json;

use super::ReadRunEvents;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, recorded, running_board,
};
use crate::application::swarm::dto::ReadRunEventsRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn page(after: serde_json::Value, limit: serde_json::Value) -> ReadRunEventsRequest {
    ReadRunEventsRequest {
        actor: "parent".to_owned(),
        after,
        limit,
    }
}

fn board() -> std::sync::Arc<MemoryBoard> {
    let mut state = running_board(100.0);
    for action in ["created", "claimed", "sent"] {
        state.events.push(recorded("parent", action, json!({})));
    }
    MemoryBoard::with(state)
}

/// A page reads one more than its limit to know whether more follow; the
/// next cursor is its last id, or the given one when it is empty.
#[test]
fn a_page_says_where_to_read_on_and_whether_more_follow() {
    let service = ReadRunEvents::new(board(), SteppingClock::fixed(1.0));
    let first = service.execute(page(json!(0), json!(2))).unwrap();
    assert_eq!(
        (first.events.len(), first.cursor, first.has_more),
        (2, 2, true)
    );
    let last = service.execute(page(json!(2), json!(2))).unwrap();
    assert_eq!(
        (last.events.len(), last.cursor, last.has_more),
        (1, 3, false)
    );
    let empty = service.execute(page(json!(9), json!(25))).unwrap();
    assert_eq!(
        (empty.events.len(), empty.cursor, empty.has_more),
        (0, 9, false)
    );
}

/// The bounds are checked before any transaction opens.
#[test]
fn the_bounds_are_checked_before_the_gate() {
    let board = board();
    let service = ReadRunEvents::new(board.clone(), SteppingClock::fixed(1.0));
    for (after, limit) in [
        (json!(-1), json!(1)),
        (json!(0), json!(0)),
        (json!(true), json!(1)),
    ] {
        assert_eq!(
            service.execute(page(after, limit)).unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "event page requires nonnegative cursor and limit 1 through 100"
            )
        );
    }
    assert!(board.transactions().is_empty());
}

#[test]
fn over_reads_the_given_board() {
    let composed_over = board();
    ReadRunEvents::new(composed_over.clone(), SteppingClock::fixed(1.0))
        .over(board())
        .execute(page(json!(0), json!(25)))
        .unwrap();
    assert!(composed_over.transactions().is_empty());
}

use serde_json::json;

use super::ReadRunSummary;
use crate::application::swarm::board_test_support::{MemoryBoard, SteppingClock, running_board};
use crate::application::swarm::dto::{ReadRunSummaryRequest, RunSummary};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn read(since: serde_json::Value) -> ReadRunSummaryRequest {
    ReadRunSummaryRequest {
        actor: "parent".to_owned(),
        since,
    }
}

/// A cursor that is not a nonnegative integer is refused before any
/// transaction opens.
#[test]
fn the_cursor_is_checked_before_the_gate() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ReadRunSummary::new(board.clone(), SteppingClock::fixed(1.0));
    for since in [json!(-1), json!(true), json!("1"), json!(1.5)] {
        assert_eq!(
            service.execute(read(since)).unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "summary cursor must be a nonnegative integer"
            )
        );
    }
    assert!(board.transactions().is_empty());
}

/// The board's cursor answers `unchanged`; any other the full summary.
#[test]
fn the_boards_cursor_answers_unchanged() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ReadRunSummary::new(board.clone(), SteppingClock::fixed(1.0));
    assert!(matches!(
        service.execute(read(json!(0))).unwrap(),
        RunSummary::Unchanged {
            event_cursor: 0,
            ..
        }
    ));
    for since in [json!(null), json!(7), json!(u64::MAX)] {
        assert!(matches!(
            service.execute(read(since)).unwrap(),
            RunSummary::Full(_)
        ));
    }
}

/// Served over another repository, the summary reads that board alone.
#[test]
fn over_reads_the_given_board() {
    let composed_over = MemoryBoard::with(running_board(100.0));
    let other = MemoryBoard::with(running_board(100.0));
    ReadRunSummary::new(composed_over.clone(), SteppingClock::fixed(1.0))
        .over(other.clone())
        .execute(read(json!(null)))
        .unwrap();
    assert!(composed_over.transactions().is_empty());
    assert_eq!(other.transactions().len(), 2, "the gate's two transactions");
}

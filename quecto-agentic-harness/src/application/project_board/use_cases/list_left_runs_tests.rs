//! #2655: the runs a session leaves open are the ones its starts launched
//! that the board still shows open, each to be ended as stopped.
use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::RunEndOutcome;
use crate::application::project_board::ports::{BoardArtifact, BoardWrite};
use crate::application::project_board::use_cases::fixtures_tests::{
    at, board, fields, person, slug,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// The runs a session's starts launched, as told.
struct Launched(Vec<(&'static str, u32)>);

impl StartedRuns for Launched {
    fn started(&self) -> Vec<(Slug, u32)> {
        self.0
            .iter()
            .map(|(task, run)| (slug(task), *run))
            .collect()
    }
}

/// A board holding task `board-store` held by Ada, run 1 going.
fn started_board() -> MemoryBoardStore {
    let store = board();
    let mut ready = fields("board-store");
    ready.accepted_plan = Some(1);
    let ready = Task::new(ready).unwrap();
    let accepted = crate::application::project_board::fold_back::tests::plan();
    let (task, run) = start(
        &ready,
        &[],
        Some(&accepted),
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap()
    .into_writes();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Task(task),
        BoardArtifact::Plan(accepted),
        BoardArtifact::Run(run),
    ];
    let written = store.write(&head, &artifacts, "board: board-store started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

fn left(store: &MemoryBoardStore, launched: Vec<(&'static str, u32)>) -> ListLeftRuns {
    ListLeftRuns::new(Arc::new(store.client()), Arc::new(Launched(launched)))
}

#[test]
fn a_run_the_session_launched_and_the_board_shows_open_is_left_to_end_as_stopped() {
    let store = started_board();
    let ends = left(&store, vec![("board-store", 1)]).execute().unwrap();
    assert_eq!(
        ends,
        [RunEnd {
            task: slug("board-store"),
            run: 1,
            outcome: RunEndOutcome::Stopped,
            expected_updated: None,
        }]
    );
}

#[test]
fn a_run_ended_unknown_or_never_launched_here_is_not_left() {
    let store = started_board();
    assert!(
        left(&store, vec![]).execute().unwrap().is_empty(),
        "nothing launched here, nothing left (another session's run 1 stays its own)"
    );
    assert!(
        left(&store, vec![("board-store", 2), ("gone", 1)])
            .execute()
            .unwrap()
            .is_empty(),
        "a run or task the board does not have"
    );
    crate::application::project_board::use_cases::EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        crate::application::project_board::use_cases::fixtures_tests::SetClock::at("10:30:00"),
        Arc::new(crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells),
        crate::application::project_board::use_cases::fixtures_tests::Remote::holding_all(),
        None,
        None,
    )
    .unwrap()
    .execute(&RunEnd {
        task: slug("board-store"),
        run: 1,
        outcome: RunEndOutcome::Failed,
        expected_updated: None,
    })
    .unwrap();
    assert!(
        left(&store, vec![("board-store", 1)])
            .execute()
            .unwrap()
            .is_empty(),
        "a run ended already"
    );
}

/// Round 1 (#2674): a run named twice among the started is owed once.
#[test]
fn a_run_named_twice_is_left_once() {
    let store = started_board();
    let ends = left(&store, vec![("board-store", 1), ("board-store", 1)])
        .execute()
        .unwrap();
    assert_eq!(ends.len(), 1, "{ends:?}");
}

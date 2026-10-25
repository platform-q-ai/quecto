//! Contract for [`BegunStarts`] (epic #2705 B8): where a begun start whose
//! coordinator did not launch is undone. Every adapter folds the run back
//! once and says where it left it, and never says a run it could not
//! reach was folded: this machine's board (`BeginBoardRun`, which also
//! times it), and a board server's (`RemoteBoardRuns`), which reads no
//! clock.
use std::sync::Arc;

use quecto::application::project_board::dto::{RunAfterUndo, TaskStart};
use quecto::application::project_board::ports::{BegunStarts, ProjectBoardStore};
use quecto::application::project_board::start::StartUndo;
use quecto::application::project_board::use_cases::BeginBoardRun;
use quecto::domain::project_board::entities::claim::Identity;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::telemetry::BoardId;
use quecto::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus};
use quecto::infrastructure::http::board_remote::{RemoteBoardOperations, endpoint_of};
use quecto::infrastructure::http::board_remote_runs::RemoteBoardRuns;
use quecto::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use quecto::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use quecto::infrastructure::time::board_clock::SystemBoardClock;

use super::coordinator_launch::cancel::ready_board;

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

fn undo_of(run: u32) -> StartUndo {
    StartUndo {
        task: Slug::parse("a").expect("a slug"),
        plan_version: 1,
        run,
        from: TaskStatus::Ready,
        reason: "no container runtime".into(),
    }
}

#[test]
fn a_begun_start_is_undone_once_on_this_machines_board() {
    let store = MemoryBoardStore::default();
    ready_board(&store.client_as(ada()));
    let ada: Arc<dyn ProjectBoardStore> = Arc::new(store.client_as(ada()));
    let begin = BeginBoardRun::new(
        Arc::clone(&ada),
        ada,
        Arc::new(SystemBoardClock::default()),
        Arc::new(MarkdownTextCells),
    )
    .expect("a begin");
    let start = TaskStart {
        task: Slug::parse("a").expect("a slug"),
        plan_version: 1,
    };
    let begun = begin.begin(&start, None).expect("begun");
    assert_eq!((begun.launch.run, begun.from), (1, TaskStatus::Ready));
    let undo = begun.undo("no container runtime");
    assert_eq!(undo, undo_of(1));
    let runs: &dyn BegunStarts = &begin;
    let (state, folded, refused) = runs.undo(&undo, true);
    assert_eq!(refused, None);
    assert_eq!(state, RunAfterUndo::Folded);
    assert!(
        folded.is_some_and(|folded| folded.result.is_ok()),
        "its fold-back"
    );
    let board = store.read().expect("read");
    assert_eq!(board.tasks[0].fields().status, TaskStatus::Ready);
    assert_eq!(board.runs[0].fields().outcome, Some(RunOutcome::Failed));
    assert_eq!(runs.undo(&undo, false).0, RunAfterUndo::EndedAlready);
    assert_eq!(
        runs.undo(&undo_of(2), false).0,
        RunAfterUndo::Open,
        "no run 2"
    );
    assert!(runs.elapsed().is_some(), "timed on its clock");
}

#[test]
fn an_undo_a_board_server_never_answers_leaves_the_run_open() {
    let client = endpoint_of("unix:///nonexistent/board.sock", None).expect("a client");
    let board = BoardId::of_hex("0123456789abcdef").expect("a board id");
    let remote = RemoteBoardOperations::new(vec![client], board, Some(ada()));
    let runs = RemoteBoardRuns(Arc::new(remote));
    assert_eq!(
        runs.undo(&undo_of(1), true),
        (RunAfterUndo::Open, None, None)
    );
    assert_eq!(runs.elapsed(), None, "a client reads no clock");
}

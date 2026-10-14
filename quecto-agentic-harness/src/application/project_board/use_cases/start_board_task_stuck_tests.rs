//! S4c: every way a start can leave its run open with no coordinator to
//! fold it back (its undo's fold-back refused, after a failed launch or a
//! cancel) is recovered by `end_run`, and then the task leaves its holder.
//! A write followed by a crash is `end_run`'s own test.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{FreezingLauncher, confirmed, plan, ready, seeded, starter_of, task_and_runs};
use crate::application::project_board::dto::{
    BoardOp, ProjectBoardError, RunAfterUndo, RunEnd, RunEndOutcome,
};
use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, person, slug};
use crate::application::project_board::use_cases::{ChangeBoardTask, EndRun};
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A launcher whose call is cancelled while it runs, as the board freezes
/// against the undo's next write.
struct CancelledAndFrozen(MemoryBoardStore, Arc<AtomicBool>);

impl CoordinatorLaunch for CancelledAndFrozen {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        self.1.store(true, Ordering::SeqCst);
        self.0.refuse_next_write("the board is frozen");
        Err(LaunchRefused::because("the call was cancelled"))
    }
}

/// The run left open is ended by Ada with `end_run`, and the task then
/// goes back to ready.
fn recovered_by_end_run(store: &MemoryBoardStore) {
    let (task, runs) = task_and_runs(store);
    assert_eq!(task.fields().status, TaskStatus::InProgress);
    assert_eq!(runs[0].fields().outcome, None, "run 1 is stranded");
    let ending = EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:05:00"),
        Arc::new(MarkdownTextCells),
        None,
    )
    .unwrap();
    let ended = ending
        .execute(&RunEnd {
            task: slug("a"),
            run: 1,
            outcome: RunEndOutcome::Failed,
            expected_updated: Some(task.fields().updated.clone()),
        })
        .unwrap();
    assert_eq!(ended.fields().outcome, Some(RunOutcome::Failed));
    let release = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:06:00"),
    )
    .unwrap();
    let task = release
        .execute(&slug("a"), &BoardOp::Move(TaskStatus::Ready))
        .unwrap();
    assert_eq!(
        (task.fields().status, &task.fields().claim),
        (TaskStatus::Ready, &None)
    );
}

#[test]
fn a_failed_launch_whose_undo_could_not_fold_back_is_recovered() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let launcher: Arc<dyn CoordinatorLaunch> = Arc::new(FreezingLauncher(store.client()));
    let refused = starter_of(&store, Some(launcher), None).execute(&confirmed(1));
    assert!(
        matches!(
            &refused,
            Err(ProjectBoardError::LaunchFailed {
                run_state: RunAfterUndo::Open,
                ..
            })
        ),
        "{refused:?}"
    );
    recovered_by_end_run(&store);
}

#[test]
fn a_cancelled_start_whose_undo_could_not_fold_back_is_recovered() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let cancelled = Arc::new(AtomicBool::new(false));
    let launcher: Arc<dyn CoordinatorLaunch> =
        Arc::new(CancelledAndFrozen(store.client(), Arc::clone(&cancelled)));
    let refused =
        starter_of(&store, Some(launcher), Some(Arc::clone(&cancelled))).execute(&confirmed(1));
    assert_eq!(
        refused,
        Err(ProjectBoardError::LaunchFailed {
            reason: "the call was cancelled".into(),
            run: 1,
            run_state: RunAfterUndo::Open,
            released: Some(false),
            restored: None,
            still_running: None,
        })
    );
    assert!(cancelled.load(Ordering::SeqCst));
    recovered_by_end_run(&store);
}

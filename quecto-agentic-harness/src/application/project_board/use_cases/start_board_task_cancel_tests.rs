//! S4c: a start whose call is cancelled launches nothing once it sees the
//! cancel, and its undo writes past the cancel, so no run is stranded.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Launcher, confirmed, plan, ready, seeded, task_and_runs};
use crate::application::project_board::dto::{ProjectBoardError, RunAfterUndo};
use crate::application::project_board::ports::BoardArtifact;
use crate::application::project_board::ports::{
    BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite, CoordinatorLaunch,
    CoordinatorRequest, LaunchRefused, LaunchedCoordinator, ProjectBoardStore, SetAsideClear,
};
use crate::application::project_board::use_cases::StartBoardTask;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, person};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A store that, like the git store, refuses every write once its call is
/// cancelled; armed, it cancels the call right after its next write lands.
struct Cancellable {
    inner: MemoryBoardStore,
    cancelled: Arc<AtomicBool>,
    cancel_after_write: AtomicBool,
}

impl ProjectBoardStore for Cancellable {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        self.inner.read()
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Ok(BoardWrite::Refused("the call was cancelled".into()));
        }
        let written = self.inner.write(expected, artifacts, message);
        if self.cancel_after_write.swap(false, Ordering::SeqCst) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        written
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        self.inner.clear_set_aside(expected, clear, message)
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        self.inner.bootstrap()
    }

    fn writer(&self) -> Identity {
        self.inner.writer()
    }
}

/// A start by Ada whose own writes go through a [`Cancellable`] store
/// over `store` (armed when `cancel_after_write`), its undo's straight to
/// `store`, launching with `launcher`.
fn cancellable_starter(
    store: &MemoryBoardStore,
    cancelled: &Arc<AtomicBool>,
    cancel_after_write: bool,
    launcher: Arc<dyn CoordinatorLaunch>,
) -> StartBoardTask {
    let own = Cancellable {
        inner: store.client_as(person("Ada")),
        cancelled: Arc::clone(cancelled),
        cancel_after_write: AtomicBool::new(cancel_after_write),
    };
    StartBoardTask::new(
        Arc::new(own),
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:00:00"),
        Arc::new(MarkdownTextCells),
        Some(launcher),
        Some(Arc::clone(cancelled)),
    )
    .unwrap()
}

/// The board as an undone start left it: task `a` ready and unheld, run 1
/// folded back as failed.
fn assert_undone(store: &MemoryBoardStore) {
    let (task, runs) = task_and_runs(store);
    assert_eq!(
        (task.fields().status, &task.fields().claim),
        (TaskStatus::Ready, &None)
    );
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fields().outcome, Some(RunOutcome::Failed));
}

/// Cancelled after its write but before its launch (the write committed,
/// then the call stopped, as a crash would): nothing is launched, and the
/// undo still folds run 1 back and releases the claim.
#[test]
fn a_start_cancelled_before_its_launch_launches_nothing_and_is_undone() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let cancelled = Arc::new(AtomicBool::new(false));
    let launcher = Launcher::launching();
    let starting = cancellable_starter(&store, &cancelled, true, launcher.clone());
    assert_eq!(
        starting.execute(&confirmed(1)),
        Err(ProjectBoardError::LaunchFailed {
            reason: "the call was cancelled before the coordinator launched".into(),
            run: 1,
            run_state: RunAfterUndo::Folded,
            released: Some(true),
            still_running: None,
        })
    );
    assert!(cancelled.load(Ordering::SeqCst));
    assert!(launcher.asked().is_empty(), "nothing launched");
    assert_undone(&store);
}

// A cancel during the launch, through the real launcher and a spawn that
// never answers, is the coordinator launch contract's
// (`a_start_cancelled_during_its_launch_drops_it_and_is_undone`).

/// Cancelled before it writes: the store refuses the write, so nothing is
/// written and nothing launched.
#[test]
fn a_start_cancelled_before_its_write_writes_nothing() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let writes = store.writes();
    let cancelled = Arc::new(AtomicBool::new(true));
    let launcher = Launcher::launching();
    let starting = cancellable_starter(&store, &cancelled, false, launcher.clone());
    let refused = starting.execute(&confirmed(1));
    assert!(
        matches!(&refused, Err(error) if !matches!(error, ProjectBoardError::LaunchFailed { .. })),
        "{refused:?}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert!(launcher.asked().is_empty(), "nothing launched");
}

/// A launcher that panics as it launches.
struct Panicking;

impl CoordinatorLaunch for Panicking {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        panic!("the launch broke");
    }
}

/// Review round 2: a launch that panics is undone as one that failed, so
/// the start's write is never left behind.
#[test]
fn a_launch_that_panics_is_undone() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let starting = super::starter_of(&store, Some(Arc::new(Panicking)), None);
    assert_eq!(
        starting.execute(&confirmed(1)),
        Err(ProjectBoardError::LaunchFailed {
            reason: "the coordinator's launch failed unexpectedly (it panicked)".into(),
            run: 1,
            run_state: RunAfterUndo::Folded,
            released: Some(true),
            still_running: None,
        })
    );
    assert_undone(&store);
}

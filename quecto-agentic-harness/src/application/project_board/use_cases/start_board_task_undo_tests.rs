//! S4c review round 3: what a failed launch's undo leaves when another
//! writer ended the run first, and when the coordinator may still be
//! running (it came up wrong and could not be stopped): then nothing is
//! undone.
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::{confirmed, plan, ready, seeded, starter_of, task_and_runs};
use crate::application::project_board::dto::{ProjectBoardError, RunAfterUndo};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::{
    CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::FoldBackRun;
use crate::application::project_board::use_cases::fixtures_tests::{SetClock, slug};
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A launcher that fails, its run having been folded back meanwhile by
/// another writer (the swarm's keeper, say) as stopped.
struct EndedMeanwhile(MemoryBoardStore);

impl CoordinatorLaunch for EndedMeanwhile {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        request: &CoordinatorRequest,
        _: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        let folding = FoldBackRun::new(
            Arc::new(self.0.client()),
            SetClock::at("10:00:30"),
            Arc::new(MarkdownTextCells),
        )
        .unwrap();
        let stopped = SwarmRunView {
            end: SwarmEnd::Cancelled,
            items: vec![],
            observed_tokens: 0,
        };
        folding
            .execute(&request.task, request.run, &stopped)
            .unwrap();
        Err(LaunchRefused::because("no container runtime"))
    }
}

#[test]
fn a_run_another_writer_ended_is_left_as_it_ended_and_the_claim_released() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let launcher: Arc<dyn CoordinatorLaunch> = Arc::new(EndedMeanwhile(store.client()));
    let refused = starter_of(&store, Some(launcher), None).execute(&confirmed(1));
    assert_eq!(
        refused,
        Err(ProjectBoardError::LaunchFailed {
            reason: "no container runtime".into(),
            run: 1,
            run_state: RunAfterUndo::EndedAlready,
            released: Some(true),
            restored: None,
            still_running: None,
        })
    );
    let (task, runs) = task_and_runs(&store);
    assert_eq!(task.fields().status, TaskStatus::Ready);
    assert_eq!(
        runs[0].fields().outcome,
        Some(RunOutcome::Stopped),
        "as it ended"
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "the coordinator did not launch: no container runtime; run 1 had already ended, and \
         the task is ready again"
    );
}

/// A launcher whose coordinator came up wrong and could not be stopped.
struct LeftRunning;

impl CoordinatorLaunch for LeftRunning {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        Err(LaunchRefused::leaving(
            "the coordinator came up outside a container, so stopping it timed out",
            "7f3a",
        ))
    }
}

/// Never fold back or release under a coordinator that may still run: the
/// run stays open for end_run once it is stopped, the claim held, and no
/// undo fold-back is recorded.
#[test]
fn a_coordinator_that_may_still_run_leaves_the_start_as_it_is() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let launcher: Arc<dyn CoordinatorLaunch> = Arc::new(LeftRunning);
    let mut facts = OpFacts::default();
    let refused =
        starter_of(&store, Some(launcher), None).execute_noted(&confirmed(1), Some(&mut facts));
    assert_eq!(
        refused.as_ref().map(|_| ()),
        Err(&ProjectBoardError::LaunchFailed {
            reason: "the coordinator came up outside a container, so stopping it timed out".into(),
            run: 1,
            run_state: RunAfterUndo::Open,
            released: Some(false),
            restored: None,
            still_running: Some("7f3a".into()),
        })
    );
    let (task, runs) = task_and_runs(&store);
    assert_eq!(task.fields().status, TaskStatus::InProgress);
    assert!(task.fields().claim.is_some(), "the claim is held");
    assert_eq!(runs[0].fields().outcome, None, "run 1 is open");
    let start = facts.start.unwrap();
    assert_eq!(
        (start.run_state, start.released, start.fold_back.is_none()),
        (Some(RunAfterUndo::Open), Some(false), true)
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "the coordinator did not launch: the coordinator came up outside a container, so \
         stopping it timed out; coordinator 7f3a may still be running, so run 1 is left open \
         and you still hold the task: stop it with agent_cmd kill 7f3a, then end the run with \
         end_run(task, run 1, outcome failed)"
    );
    assert_eq!(slug("a"), task.fields().id);
}

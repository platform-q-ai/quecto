//! #2655: a one-shot parent's reply ended while its board run was open;
//! the process exited, its coordinator went, and run 1 stayed `running`
//! with no swarm behind it. As the session ends, the harness ends each run
//! its starts launched that the board still shows open, as `stopped`,
//! through the same path a model's `end_run` takes, recorded as the
//! session's end (`client: session_end`).
use std::sync::{Arc, OnceLock};

use super::BoardSite;
use super::tests::recorded_tool;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{
    QueryListings, Swarms, assembled_reporting,
};
use crate::application::project_board::ports::{
    BoardArtifact, CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
    ProjectBoardStore, RunLiveness, StartedRuns,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::environments::entities::environment_registry::EnvironmentRegistry;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{
    Client, OpKind, OpOutcome, RunOutcomeKind,
};
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::tests::Records;
use crate::interface::tools::project_board::site::{
    BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// The board every build of this test reads: task `a` held by Ada, run 1
/// going (as `start` left it), and run 2 of task `gone`, which the board
/// does not have.
fn board() -> &'static MemoryBoardStore {
    static BOARD: OnceLock<MemoryBoardStore> = OnceLock::new();
    BOARD.get_or_init(|| {
        let store = MemoryBoardStore::default();
        store.bootstrap().unwrap();
        let (task, plan) = super::end_run_tests::ready_with_plan();
        let now =
            crate::application::project_board::ports::BoardClock::now(&SystemBoardClock::default())
                .unwrap();
        let (task, run) = start(&task, &[], Some(&plan), &super::end_run_tests::ada(), &now)
            .unwrap()
            .into_writes();
        let head = store.read().unwrap().head;
        let seed = [
            BoardArtifact::Task(task),
            BoardArtifact::Plan(plan),
            BoardArtifact::Run(run),
        ];
        store.write(&head, &seed, "board: a started").unwrap();
        store
    })
}

/// A session that launched run 1 of `a` (and run 2 of `gone`), whose
/// coordinators have all stopped; it launches nothing more.
struct LeftBehind;

impl CoordinatorLaunch for LeftBehind {
    fn may_launch(&self) -> bool {
        false
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        panic!("a session's end launches nothing")
    }
}

impl RunLiveness for LeftBehind {
    fn running_coordinator(&self, _: &Slug, _: u32) -> Option<String> {
        None
    }
}

impl StartedRuns for LeftBehind {
    fn started(&self) -> Vec<(Slug, u32)> {
        vec![
            (Slug::parse("a").unwrap(), 1),
            (Slug::parse("gone").unwrap(), 2),
        ]
    }
}

fn left_board(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = board();
    let launcher = environment.launcher.clone();
    let operations = assembled_reporting(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
        Swarms {
            launcher: launcher.clone().map(|swarms| swarms as _),
            started: launcher.clone().map(|swarms| swarms as _),
            liveness: launcher.map(|swarms| swarms as _),
            reports: None,
        },
    )
    .map_err(|error| BuildError::Unavailable(error.to_string()))?;
    Ok(ProjectBoardHandles { operations })
}

#[test]
fn a_run_its_session_left_open_is_ended_as_stopped_and_recorded_so() {
    let records = Arc::new(Records::default());
    let tool = recorded_tool(
        BoardSite::agent(
            ".".into(),
            left_board,
            Arc::new(LeftBehind),
            &EnvironmentRegistry::new(),
        ),
        &records,
    );
    assert_eq!(
        tool.end_left_runs(),
        ["board: run 1 of task a was still open as this session ended: ended it as stopped"]
    );
    let run = board().read().unwrap().runs[0].clone();
    let fields = run.fields();
    assert_eq!(fields.outcome, Some(RunOutcome::Stopped));
    assert!(
        fields
            .items
            .iter()
            .all(|item| item.state == ItemState::Todo),
        "no swarm board kept: every item todo"
    );
    let records = records.taken();
    assert_eq!(records.len(), 1, "one end, recorded: {records:?}");
    let record = &records[0];
    assert_eq!(
        (
            record.op,
            record.client,
            &record.outcome,
            record.run,
            record.run_outcome
        ),
        (
            OpKind::EndRun,
            Client::SessionEnd,
            &OpOutcome::Ok,
            Some(1),
            Some(RunOutcomeKind::Stopped)
        )
    );
    assert!(
        tool.end_left_runs().is_empty(),
        "a run ended is never ended again"
    );
}

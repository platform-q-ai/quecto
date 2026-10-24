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
use crate::application::project_board::ports::{
    BoardArtifact, CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
    ProjectBoardStore, RunLiveness, StartedRuns,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::composition::bootstrap::project_board_operations::fixtures::{
    QueryListings, Swarms, assembled_reporting,
};
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
        let (task, run) = start(
            &task,
            &[],
            &[],
            Some(&plan),
            &super::end_run_tests::ada(),
            &now,
        )
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
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

/// A board on which task `a`'s run 1 is held by Bob, not by the session's
/// writer (Ada): her end is refused.
fn bobs_board() -> &'static MemoryBoardStore {
    static BOARD: OnceLock<MemoryBoardStore> = OnceLock::new();
    BOARD.get_or_init(|| {
        let store = MemoryBoardStore::default();
        store.bootstrap().unwrap();
        let (task, plan) = super::end_run_tests::ready_with_plan();
        let now =
            crate::application::project_board::ports::BoardClock::now(&SystemBoardClock::default())
                .unwrap();
        let bob = crate::domain::project_board::entities::claim::Identity {
            name: "Bob".into(),
            email: "bob@example.com".into(),
        };
        let (task, run) = start(&task, &[], &[], Some(&plan), &bob, &now)
            .unwrap()
            .into_writes();
        let head = store.read().unwrap().head;
        let seed = [
            BoardArtifact::Task(task),
            BoardArtifact::Plan(plan),
            BoardArtifact::Run(run),
        ];
        store
            .write(&head, &seed, "board: a started by Bob")
            .unwrap();
        store
    })
}

fn bobs(environment: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    let store = bobs_board();
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
    Ok(ProjectBoardHandles {
        operations: Box::new(operations),
        board: None,
    })
}

/// Round 1 (#2674): an end the board's rules refuse says so, naming the
/// rule, and records the refusal as the session's end.
#[test]
fn a_refused_end_says_so_naming_its_rule() {
    let records = Arc::new(Records::default());
    let tool = recorded_tool(
        BoardSite::agent(
            ".".into(),
            bobs,
            Arc::new(LeftBehind),
            &EnvironmentRegistry::new(),
        ),
        &records,
    );
    assert_eq!(
        tool.end_left_runs(),
        [
            "board: run 1 of task a was still open as this session ended, and ending it as \
          stopped was refused (not_holder): end it with end_run"
        ]
    );
    assert_eq!(bobs_board().read().unwrap().runs[0].fields().outcome, None);
    let records = records.taken();
    assert_eq!(
        (records.len(), records[0].client),
        (1, Client::SessionEnd),
        "{records:?}"
    );
}

/// A session whose run 1 of `a` still has its coordinator running.
struct StillRunning;

impl CoordinatorLaunch for StillRunning {
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

impl RunLiveness for StillRunning {
    fn running_coordinator(&self, _: &Slug, _: u32) -> Option<String> {
        Some("7f3a".into())
    }
}

impl StartedRuns for StillRunning {
    fn started(&self) -> Vec<(Slug, u32)> {
        vec![(Slug::parse("a").unwrap(), 1)]
    }
}

/// A coordinator still running keeps its run open: the end is refused
/// (`run_live`) and says so.
#[test]
fn a_run_whose_coordinator_still_runs_is_left_open_and_said_so() {
    let tool = super::tests::tool(BoardSite::agent(
        ".".into(),
        bobs,
        Arc::new(StillRunning),
        &EnvironmentRegistry::new(),
    ));
    assert_eq!(
        tool.end_left_runs(),
        [
            "board: run 1 of task a was still open as this session ended, and ending it as \
          stopped was refused (run_live): end it with end_run"
        ]
    );
}

fn unreadable(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
    Err(BuildError::Unavailable("the board is gone".into()))
}

/// A board that cannot be read says the session's runs could not be.
#[test]
fn a_board_that_cannot_be_read_says_so() {
    let tool = super::tests::tool(BoardSite::agent(
        ".".into(),
        unreadable,
        Arc::new(LeftBehind),
        &EnvironmentRegistry::new(),
    ));
    assert_eq!(
        tool.end_left_runs(),
        [
            "board: the runs this session started could not be read as it ended (the board is \
          gone): end any still open with end_run"
        ]
    );
}

/// A session that has never launched a board run.
struct NeverStarted;

impl CoordinatorLaunch for NeverStarted {
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

impl RunLiveness for NeverStarted {
    fn running_coordinator(&self, _: &Slug, _: u32) -> Option<String> {
        None
    }
}

impl StartedRuns for NeverStarted {
    fn started(&self) -> Vec<(Slug, u32)> {
        vec![]
    }
}

#[test]
fn an_empty_started_ledger_ends_quietly_without_building_a_board() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    fn forbidden_build(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
        BUILDS.fetch_add(1, Ordering::SeqCst);
        Err(BuildError::Unavailable(
            "empty session built a board".into(),
        ))
    }

    let tool = super::tests::tool(BoardSite::agent(
        ".".into(),
        forbidden_build,
        Arc::new(NeverStarted),
        &EnvironmentRegistry::new(),
    ));
    let lines = tool.end_left_runs();
    assert_eq!(
        (lines, BUILDS.load(Ordering::SeqCst)),
        (Vec::<String>::new(), 0),
        "an empty session must emit nothing and never compose a board"
    );
}

#[test]
fn a_site_without_a_launcher_ends_quietly_without_building_a_board() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    fn forbidden_build(_: &ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError> {
        BUILDS.fetch_add(1, Ordering::SeqCst);
        Err(BuildError::Unavailable(
            "launcherless session built a board".into(),
        ))
    }

    let tool = super::tests::tool(BoardSite {
        repo: ".".into(),
        remote: "origin".into(),
        known: Default::default(),
        build: Some(forbidden_build),
        launcher: None,
        swarm_session: None,
    });
    let lines = tool.end_left_runs();
    assert_eq!(
        (lines, BUILDS.load(Ordering::SeqCst)),
        (Vec::<String>::new(), 0),
        "a session without a launcher must emit nothing and never compose a board"
    );
}

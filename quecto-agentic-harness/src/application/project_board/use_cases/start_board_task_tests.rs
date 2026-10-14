//! S4c: a start writes the task and run 1 together, then launches the
//! coordinator; a failed launch is undone as far as it can be.
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use super::StartBoardTask;
use crate::application::project_board::dto::{
    PlanAcceptance, ProjectBoardError, RunAfterUndo, TaskStart,
};
use crate::application::project_board::ports::{
    BoardArtifact, CoordinatorLaunch, CoordinatorRequest, LaunchRefused, LaunchedCoordinator,
    ProjectBoardStore,
};
use crate::application::project_board::swarm_brief::BriefRefusal;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::AcceptPlan;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, plan_item, slug,
};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::plan::{Plan, PlanFields};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::telemetry::StartClaim;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, PlanState, RunOutcome, TaskStatus,
};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A launcher that answers as told and keeps every request it was given;
/// one that may not launch says so when asked.
struct Launcher {
    answer: Result<LaunchedCoordinator, LaunchRefused>,
    asked: Mutex<Vec<CoordinatorRequest>>,
    may: bool,
}

impl Launcher {
    fn launching() -> Arc<Self> {
        Arc::new(Self {
            answer: Ok(LaunchedCoordinator {
                agent: "0f8e-coordinator".into(),
                environment_ref: "C7".into(),
            }),
            asked: Mutex::default(),
            may: true,
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            answer: Err(LaunchRefused::because("no container runtime")),
            asked: Mutex::default(),
            may: true,
        })
    }

    /// A session whose policy no longer allows spawn.
    fn forbidden() -> Arc<Self> {
        Arc::new(Self {
            may: false,
            ..Arc::into_inner(Self::launching()).unwrap()
        })
    }

    fn asked(&self) -> Vec<CoordinatorRequest> {
        self.asked.lock().unwrap().clone()
    }
}

impl CoordinatorLaunch for Launcher {
    fn may_launch(&self) -> bool {
        self.may
    }

    fn launch(
        &self,
        request: &CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        self.asked.lock().unwrap().push(request.clone());
        self.answer.clone()
    }
}

fn role(name: &str, model: &str, members: u16) -> Role {
    Role {
        name: slug(name),
        model: model.into(),
        effort: Effort::High,
        limits: RoleLimits {
            members,
            turns: None,
        },
    }
}

/// Version `version` of task `a`'s plan, in `state`: a coordinator and
/// two builders, two task items.
fn plan(version: u32, state: PlanState) -> Plan {
    let accepted = match state {
        PlanState::Accepted | PlanState::Superseded => Some((person("Ada"), at("09:00:00"))),
        PlanState::Draft | PlanState::Discarded => None,
    };
    Plan::new(PlanFields {
        task: slug("a"),
        version,
        state,
        approach: "Spike, then build.".into(),
        items: vec![plan_item("schema", &[]), plan_item("api", &["schema"])],
        team: Some(Team {
            roles: vec![
                role("coordinator", "anthropic/claude-opus", 1),
                role("builder", "gpt-5.5", 2),
            ],
            budget: None,
            deadline: None,
            image: None,
        }),
        proposed_by: person("Ada"),
        edited_by: None,
        accepted,
        created: at("09:00:00"),
        updated: at("09:00:00"),
    })
    .unwrap()
}

/// A board holding task `a` as `shaped`, written at 09:00, and `plans`
/// and `runs` of it.
fn seeded(
    shaped: impl FnOnce(TaskFields) -> TaskFields,
    plans: Vec<Plan>,
    runs: Vec<Run>,
) -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let task = Task::new(shaped(TaskFields {
        accepted_plan: Some(1),
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    }))
    .unwrap();
    let artifacts: Vec<BoardArtifact> = std::iter::once(BoardArtifact::Task(task))
        .chain(plans.into_iter().map(BoardArtifact::Plan))
        .chain(runs.into_iter().map(BoardArtifact::Run))
        .collect();
    store.write(&head, &artifacts, "seed").unwrap();
    store
}

fn ready(fields: TaskFields) -> TaskFields {
    fields
}

/// Task `a` in progress, held by Ada until 11:00.
fn held_by_ada(fields: TaskFields) -> TaskFields {
    TaskFields {
        status: TaskStatus::InProgress,
        claim: Some(Claim {
            holder: person("Ada"),
            since: at("09:00:00"),
            expires: at("11:00:00"),
        }),
        ..fields
    }
}

fn starter(store: &MemoryBoardStore, launcher: Option<Arc<Launcher>>) -> StartBoardTask {
    let launcher = launcher.map(|launcher| launcher as Arc<dyn CoordinatorLaunch>);
    starter_of(store, launcher, None)
}

/// A start by Ada on `store` (its undo too), launching with `launcher`,
/// cancelled by `cancelled` when given.
fn starter_of(
    store: &MemoryBoardStore,
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    cancelled: Option<Arc<AtomicBool>>,
) -> StartBoardTask {
    StartBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:00:00"),
        Arc::new(MarkdownTextCells),
        launcher,
        cancelled,
    )
    .unwrap()
}

fn confirmed(version: u32) -> TaskStart {
    TaskStart {
        task: slug("a"),
        plan_version: version,
    }
}

fn task_and_runs(store: &MemoryBoardStore) -> (Task, Vec<Run>) {
    let board = store.read().unwrap();
    (board.tasks[0].clone(), board.runs)
}

#[test]
fn a_launched_start_leaves_the_task_in_progress_with_run_1_open() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let launcher = Launcher::launching();
    let mut op = OpFacts::default();
    let started = starter(&store, Some(launcher.clone()))
        .execute_noted(&confirmed(1), Some(&mut op))
        .unwrap();
    let facts = op.start.clone().expect("a start notes its facts");
    assert_eq!(
        (
            &started.coordinator.agent,
            &started.coordinator.environment_ref
        ),
        (&"0f8e-coordinator".to_string(), &"C7".to_string())
    );
    assert_eq!(started.claim, StartClaim::Fresh);
    let (task, runs) = task_and_runs(&store);
    assert_eq!(task, started.task);
    assert_eq!(task.fields().status, TaskStatus::InProgress);
    assert!(
        task.fields()
            .claim
            .as_ref()
            .unwrap()
            .holder
            .is(&person("Ada"))
    );
    assert_eq!(runs, vec![started.run.clone()]);
    let run = runs[0].fields();
    assert_eq!((run.number, run.plan_version, run.outcome), (1, 1, None));
    let asked = launcher.asked();
    assert_eq!(asked.len(), 1);
    let request = &asked[0];
    assert_eq!(
        (
            request.run,
            request.plan_version,
            request.model.as_deref(),
            request.effort
        ),
        (1, 1, Some("anthropic/claude-opus"), Some(Effort::High))
    );
    assert!(
        request.brief.contains(r#""title":"schema: Item schema""#),
        "{}",
        request.brief
    );
    assert!(
        request.brief.contains(r#""member_limit":3"#),
        "{}",
        request.brief
    );
    assert_eq!(
        started.not_applied.len(),
        1,
        "the builders' model is not applied"
    );
    assert_eq!(
        (
            facts.run,
            facts.plan_version,
            facts.members,
            facts.roles,
            facts.items
        ),
        (Some(1), Some(1), Some(3), Some(2), Some(2))
    );
    assert_eq!(
        (facts.not_applied, facts.claim),
        (Some(1), Some(StartClaim::Fresh))
    );
    // The clock stands still: the launch took no time on it.
    assert_eq!(facts.launch_us, Some(0));
    assert_eq!(
        (facts.run_state, facts.released, &facts.fold_back),
        (None, None, &None)
    );
    assert_eq!(op.plan_version, Some(1));
    assert_eq!(
        (op.attempts, op.status_from, op.status_to),
        (
            Some(1),
            Some(TaskStatus::Ready),
            Some(TaskStatus::InProgress)
        )
    );
}

/// The claim taken by a start whose launch failed is given back: run 1 is
/// folded back as failed first (a held task leaves its holder only once
/// its run ended), then the task is ready again.
#[test]
fn a_failed_launch_folds_run_1_back_as_failed_and_releases_the_claim() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let refused = starter(&store, Some(Launcher::failing())).execute(&confirmed(1));
    assert_eq!(
        refused,
        Err(ProjectBoardError::LaunchFailed {
            reason: "no container runtime".into(),
            run: 1,
            run_state: RunAfterUndo::Folded,
            released: Some(true),
            restored: None,
            still_running: None,
        })
    );
    let (task, runs) = task_and_runs(&store);
    assert_eq!(
        (task.fields().status, &task.fields().claim),
        (TaskStatus::Ready, &None)
    );
    let run = runs[0].fields();
    assert_eq!((run.number, run.outcome), (1, Some(RunOutcome::Failed)));
    assert!(run.ended.is_some());
}

/// A task its starter held before keeps its claim when the launch fails:
/// only the run is folded back.
#[test]
fn a_failed_launch_of_a_held_task_keeps_the_claim() {
    let store = seeded(held_by_ada, vec![plan(1, PlanState::Accepted)], vec![]);
    let refused = starter(&store, Some(Launcher::failing())).execute(&confirmed(1));
    assert!(
        matches!(
            &refused,
            Err(ProjectBoardError::LaunchFailed {
                run_state: RunAfterUndo::Folded,
                released: None,
                ..
            })
        ),
        "{refused:?}"
    );
    let (task, runs) = task_and_runs(&store);
    assert_eq!(task.fields().status, TaskStatus::InProgress);
    assert!(task.fields().claim.is_some());
    assert_eq!(runs[0].fields().outcome, Some(RunOutcome::Failed));
}

/// With no launcher (a session that may not spawn) nothing is read or
/// written, and with nothing accepted, a run open or a plan that cannot
/// brief a swarm nothing is written or launched.
#[test]
fn a_start_that_cannot_launch_writes_nothing() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let writes = store.writes();
    assert_eq!(
        starter(&store, None).execute(&confirmed(1)),
        Err(ProjectBoardError::NoLauncher)
    );
    let forbidden = Launcher::forbidden();
    assert_eq!(
        starter(&store, Some(forbidden.clone())).execute(&confirmed(1)),
        Err(ProjectBoardError::NoLauncher),
        "a session whose policy stopped allowing spawn starts nothing"
    );
    assert!(forbidden.asked().is_empty());
    let unaccepted = seeded(
        |fields| TaskFields {
            accepted_plan: None,
            ..fields
        },
        vec![plan(1, PlanState::Draft)],
        vec![],
    );
    let open = {
        let task = Task::new(held_by_ada(TaskFields {
            accepted_plan: Some(1),
            created: at("09:00:00"),
            updated: at("09:00:00"),
            ..fields("a")
        }))
        .unwrap();
        let (_, run) = crate::domain::project_board::services::starts::start(
            &task,
            &[],
            Some(&plan(1, PlanState::Accepted)),
            &person("Ada"),
            &at("09:30:00"),
        )
        .unwrap()
        .into_writes();
        seeded(held_by_ada, vec![plan(1, PlanState::Accepted)], vec![run])
    };
    let mut itemless = plan(1, PlanState::Accepted).into_fields();
    itemless.items = vec![];
    let unbriefed = seeded(ready, vec![Plan::new(itemless).unwrap()], vec![]);
    let cases = [
        (
            unaccepted,
            ProjectBoardError::Rule(BoardRuleError::NoAcceptedPlan),
        ),
        (open, ProjectBoardError::Rule(BoardRuleError::RunOpen(1))),
        (
            unbriefed,
            ProjectBoardError::from(BriefRefusal::NoTaskItems),
        ),
    ];
    for (store, refusal) in cases {
        let launcher = Launcher::launching();
        let writes = store.writes();
        assert_eq!(
            starter(&store, Some(launcher.clone())).execute(&confirmed(1)),
            Err(refusal)
        );
        assert_eq!(store.writes(), writes, "nothing written");
        assert!(launcher.asked().is_empty(), "nothing launched");
    }
    assert_eq!(store.writes(), writes);
}

/// A brief the plan's text grows past a coordinator's room once escaped
/// (quotes take two bytes each in its JSON) is refused while it is built,
/// before anything is written: never after the start's write.
#[test]
fn a_brief_escaped_past_its_room_is_refused_before_any_write() {
    use crate::domain::project_board::entities::plan::PlanItem;
    let quoted = "\"".repeat(260);
    let mut fields = plan(1, PlanState::Accepted).into_fields();
    fields.items = (0..14)
        .map(|n| PlanItem {
            id: slug(&format!("item-{n}")),
            title: format!("Item {n}"),
            criteria: (0..15).map(|c| format!("{c}{quoted}")).collect(),
            depends_on: vec![],
        })
        .collect();
    let store = seeded(ready, vec![Plan::new(fields).unwrap()], vec![]);
    let writes = store.writes();
    let launcher = Launcher::launching();
    let refused = starter(&store, Some(launcher.clone())).execute(&confirmed(1));
    assert!(
        matches!(
            &refused,
            Err(ProjectBoardError::Brief {
                code: "brief_too_large",
                ..
            })
        ),
        "{refused:?}"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert!(launcher.asked().is_empty(), "nothing launched");
}

/// The plan launched is the one confirmed: an acceptance that lands
/// between the confirmation and the start (here, while it is written) is
/// refused on the retry, and nothing is launched.
#[test]
fn a_start_racing_an_acceptance_launches_nothing_unconfirmed() {
    let store = seeded(
        ready,
        vec![plan(1, PlanState::Accepted), plan(2, PlanState::Draft)],
        vec![],
    );
    let rival = store.client_as(person("Bob"));
    store.after_next_read(move |_| {
        let accepting = AcceptPlan::new(Arc::new(rival), SetClock::at("09:30:00")).unwrap();
        accepting
            .execute(&PlanAcceptance {
                task: slug("a"),
                version: 2,
                expected_updated: at("09:00:00"),
                expected_accepted: Some(1),
            })
            .unwrap();
    });
    let launcher = Launcher::launching();
    let mut facts = OpFacts::default();
    let refused =
        starter(&store, Some(launcher.clone())).execute_noted(&confirmed(1), Some(&mut facts));
    assert_eq!(
        refused.map(|_| ()),
        Err(ProjectBoardError::Rule(
            BoardRuleError::AcceptedPlanChanged { accepted: Some(2) }
        ))
    );
    assert!(launcher.asked().is_empty(), "nothing launched");
    let (task, runs) = task_and_runs(&store);
    assert_eq!((task.fields().status, runs.len()), (TaskStatus::Ready, 0));
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!(facts.start.and_then(|start| start.run), None);
}

/// A launcher that fails, and while it runs, freezes the board against
/// the next write: the start's undo then cannot fold its run back, and
/// says so, leaving the claim it took (it cannot leave with a run open).
struct FreezingLauncher(MemoryBoardStore);

impl CoordinatorLaunch for FreezingLauncher {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused> {
        self.0.refuse_next_write("the board is frozen");
        Err(LaunchRefused::because("no container runtime"))
    }
}

#[test]
fn an_undo_that_cannot_fold_back_says_so_and_keeps_the_claim() {
    let store = seeded(ready, vec![plan(1, PlanState::Accepted)], vec![]);
    let launcher: Arc<dyn CoordinatorLaunch> = Arc::new(FreezingLauncher(store.client()));
    let starting = starter_of(&store, Some(launcher), None);
    let refused = starting.execute(&confirmed(1));
    assert_eq!(
        refused,
        Err(ProjectBoardError::LaunchFailed {
            reason: "no container runtime".into(),
            run: 1,
            run_state: RunAfterUndo::Open,
            released: Some(false),
            restored: None,
            still_running: None,
        })
    );
    let (task, runs) = task_and_runs(&store);
    assert_eq!(task.fields().status, TaskStatus::InProgress);
    assert_eq!(runs[0].fields().outcome, None, "run 1 is still open");
    let message = refused.unwrap_err().to_string();
    assert_eq!(
        message,
        "the coordinator did not launch: no container runtime; run 1 could not be folded back: \
         it is still open; end it with end_run(task, run 1, outcome failed), but the task could \
         not be moved back to ready: you still hold it; once its run has ended, move it with \
         move(task, to ready)"
    );
}

#[path = "start_board_task_cancel_tests.rs"]
mod cancel;

#[path = "start_board_task_undo_tests.rs"]
mod undo;

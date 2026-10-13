//! `QueryBoard::show` (#2481 M4a): a task with its plans and runs, read
//! from one snapshot of the board; the accepted plan only where the task
//! and the plan agree; a plan or run file set aside named, never a
//! refusal or a panic; an artifact the board holds twice refused.
use super::{QueryBoard, shown_on};
use crate::application::project_board::dto::{ProjectBoardError, ShownTask};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardSnapshot, BoardWrite, ProjectBoardStore,
    QuarantinedFile,
};
use crate::application::project_board::use_cases::fixtures_tests::{board, fields, slug};
use crate::domain::project_board::entities::plan::{Plan, PlanFields};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::PlanState;
use crate::infrastructure::test_support::board_artifacts::{plan, run};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

/// Plan `version` of `task`, in `state`.
fn plan_in(task: &str, version: u32, state: PlanState) -> Plan {
    let base = plan(task, version).into_fields();
    let accepted = match state {
        PlanState::Accepted | PlanState::Superseded => {
            Some((base.proposed_by.clone(), base.created.clone()))
        }
        PlanState::Draft | PlanState::Discarded => None,
    };
    Plan::new(PlanFields {
        state,
        accepted,
        ..base
    })
    .unwrap()
}

/// Run `number` of `task`'s plan `version`.
fn run_of(task: &str, number: u32, version: u32) -> Run {
    Run::new(RunFields {
        plan_version: version,
        ..run(task, number).into_fields()
    })
    .unwrap()
}

/// Task `id`, naming `accepted` as its accepted plan.
fn task_naming(id: &str, accepted: Option<u32>) -> Task {
    Task::new(TaskFields {
        accepted_plan: accepted,
        ..fields(id)
    })
    .unwrap()
}

/// A board holding `artifacts`, written over the bootstrap.
fn holding(artifacts: Vec<BoardArtifact>) -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

/// Task `a` naming plan 2, accepted after plan 1, with runs of both plans
/// (and of plan 9, which the board does not hold) written out of order;
/// task `b` with a plan and a run of its own.
fn seeded() -> MemoryBoardStore {
    holding(vec![
        BoardArtifact::Task(task_naming("a", Some(2))),
        BoardArtifact::Task(task_naming("b", None)),
        BoardArtifact::Plan(plan_in("a", 2, PlanState::Accepted)),
        BoardArtifact::Plan(plan_in("a", 1, PlanState::Superseded)),
        BoardArtifact::Plan(plan_in("b", 1, PlanState::Draft)),
        BoardArtifact::Run(run_of("a", 3, 2)),
        BoardArtifact::Run(run_of("a", 1, 1)),
        BoardArtifact::Run(run_of("a", 2, 9)),
        BoardArtifact::Run(run_of("b", 1, 1)),
    ])
}

fn shown(store: &MemoryBoardStore, id: &str) -> Result<ShownTask, ProjectBoardError> {
    QueryBoard::new(Arc::new(store.client())).show(&slug(id))
}

fn versions(shown: &ShownTask) -> Vec<u32> {
    shown
        .plans
        .iter()
        .map(|plan| plan.fields().version)
        .collect()
}

fn numbers(shown: &ShownTask) -> Vec<u32> {
    shown.runs.iter().map(|run| run.fields().number).collect()
}

#[test]
fn show_gives_the_task_with_its_plans_and_runs_in_order() {
    let store = seeded();
    let a = shown(&store, "a").unwrap();
    assert_eq!(a.head, store.read().unwrap().head, "one snapshot's head");
    assert_eq!(a.task, task_naming("a", Some(2)));
    assert_eq!(versions(&a), [1, 2]);
    // Every run of the task, whichever plan it ran.
    assert_eq!(numbers(&a), [1, 2, 3]);
    assert!(a.runs.iter().all(|run| run.fields().task == slug("a")));
    assert_eq!(
        a.accepted_plan(),
        Some(&plan_in("a", 2, PlanState::Accepted))
    );
    assert_eq!(a.latest_run().map(|run| run.fields().number), Some(3));
    assert!(a.set_aside.is_empty());
    assert!(!a.may_be_incomplete());
    let b = shown(&store, "b").unwrap();
    assert_eq!((versions(&b), numbers(&b)), (vec![1], vec![1]));
    assert_eq!(b.accepted_plan(), None, "a draft is not the task's plan");
}

#[test]
fn a_task_with_no_plan_or_run_shows_none() {
    let store = holding(vec![BoardArtifact::Task(task_naming("a", None))]);
    let a = shown(&store, "a").unwrap();
    assert_eq!((a.plans.len(), a.runs.len()), (0, 0));
    assert_eq!((a.accepted_plan(), a.latest_run()), (None, None));
    assert!(!a.may_be_incomplete());
}

/// The accepted plan is shown only where the task and the plan agree: a
/// task naming a version the board holds as a draft, or a version it does
/// not hold, shows none; so does an accepted plan the task does not name.
#[test]
fn the_accepted_plan_is_shown_only_where_the_task_and_plan_agree() {
    let named_draft = holding(vec![
        BoardArtifact::Task(task_naming("a", Some(1))),
        BoardArtifact::Plan(plan_in("a", 1, PlanState::Draft)),
    ]);
    assert_eq!(shown(&named_draft, "a").unwrap().accepted_plan(), None);
    let named_missing = holding(vec![
        BoardArtifact::Task(task_naming("a", Some(4))),
        BoardArtifact::Plan(plan_in("a", 1, PlanState::Superseded)),
    ]);
    assert_eq!(shown(&named_missing, "a").unwrap().accepted_plan(), None);
    let unnamed = holding(vec![
        BoardArtifact::Task(task_naming("a", None)),
        BoardArtifact::Plan(plan_in("a", 1, PlanState::Accepted)),
    ]);
    assert_eq!(shown(&unnamed, "a").unwrap().accepted_plan(), None);
}

/// A plan or run file of the task set aside is named, and the task still
/// shown: what it held is unknown, so the plans and runs may be
/// incomplete. Another task's file, and a stray the next write drops,
/// leave it complete; a kept file whose name spells no key may hold
/// anything, so it counts.
#[test]
fn a_set_aside_plan_or_run_is_named_never_refused() {
    let store = seeded();
    store.quarantine_key(ArtifactKey::Plan(slug("a"), 2), "plans/a.2.md", "malformed");
    store.quarantine_key(ArtifactKey::Run(slug("a"), 3), "runs/a.3.md", "malformed");
    store.quarantine_key(ArtifactKey::Plan(slug("b"), 1), "plans/b.1.md", "malformed");
    store.set_aside_stray("runs/notes.txt", "not a board file");
    let a = shown(&store, "a").unwrap();
    let paths: Vec<&str> = a.set_aside.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["plans/a.2.md", "runs/a.3.md"]);
    assert!(a.may_be_incomplete());
    assert_eq!((versions(&a), numbers(&a)), (vec![1], vec![1, 2]));
    assert_eq!(a.accepted_plan(), None, "the task's plan is set aside");
    let b = shown(&store, "b").unwrap();
    assert_eq!(b.set_aside.len(), 1, "its own plan only");
    store.set_aside_path("plans/a.x.md", "names no plan");
    let a = shown(&store, "a").unwrap();
    assert_eq!(a.set_aside.len(), 3, "a kept keyless file counts");
    assert!(
        a.set_aside
            .iter()
            .all(|file| file.key.is_some() || file.carried),
        "{:?}",
        a.set_aside
    );
}

#[test]
fn show_refuses_a_task_set_aside_or_not_on_the_board() {
    let store = seeded();
    assert_eq!(
        shown(&store, "c").map(|_| ()),
        Err(ProjectBoardError::NotFound(slug("c")))
    );
    store.quarantine("a", "malformed");
    assert!(matches!(
        shown(&store, "a"),
        Err(ProjectBoardError::Quarantined {
            key: ArtifactKey::Task(_),
            ..
        })
    ));
}

/// A board holding `artifacts` as a store that broke its contract would
/// hand them over: no store's read could, as each asserts it.
fn broken(plans: Vec<Plan>, runs: Vec<Run>) -> BoardSnapshot {
    BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![task_naming("a", None)],
        plans,
        runs,
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined: Vec::<QuarantinedFile>::new(),
    }
}

/// An artifact found twice is refused, naming it, rather than pick one:
/// a run held twice of two plan versions too.
#[test]
fn a_duplicate_plan_or_run_is_refused() {
    let a = slug("a");
    let duplicate = |key: ArtifactKey| Err(ProjectBoardError::Duplicate { key });
    let plans = vec![plan("a", 1), plan("a", 1)];
    assert_eq!(
        shown_on(&broken(plans, vec![]), &a).map(|_| ()),
        duplicate(ArtifactKey::Plan(a.clone(), 1))
    );
    let runs = vec![run_of("a", 2, 1), run_of("a", 2, 3)];
    assert_eq!(
        shown_on(&broken(vec![], runs), &a).map(|_| ()),
        duplicate(ArtifactKey::Run(a.clone(), 2))
    );
    let other = vec![plan("b", 1), plan("b", 1)];
    assert!(
        shown_on(&broken(other, vec![]), &a).is_ok(),
        "another task's"
    );
}

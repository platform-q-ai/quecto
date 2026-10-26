//! Epic #2705 B8 review round 2: a board server's undo of a start its
//! client began and could not launch, by each of its arms, read in the
//! server's own write.
use super::*;
use crate::application::project_board::dto::SwarmRunView;
use crate::application::project_board::fold_back::SwarmEnd;
use crate::application::project_board::start::StartUndo;
use crate::application::project_board::use_cases::{BeginBoardRun, FoldBackRun};
use crate::domain::project_board::entities::run::RunFields;

/// The server's halves of a start, writing as `who` at `when`.
fn served_as(store: &MemoryBoardStore, who: &str, when: &str) -> BeginBoardRun {
    let client = || Arc::new(store.client_as(person(who))) as Arc<dyn ProjectBoardStore>;
    BeginBoardRun::new(
        client(),
        client(),
        SetClock::at(when),
        Arc::new(MarkdownTextCells),
    )
    .unwrap()
}

/// Task `a`, as `shaped`, begun by Ada at 10:00, and the undo of its run.
fn begun(shaped: impl FnOnce(TaskFields) -> TaskFields) -> (MemoryBoardStore, StartUndo) {
    let store = seeded(shaped, vec![plan(1, PlanState::Accepted)], vec![]);
    let begun = served_as(&store, "Ada", "10:00:00").begin(&confirmed(1), None);
    (store, begun.unwrap().undo("no container runtime"))
}

fn launch_failed(run_state: RunAfterUndo, released: Option<bool>) -> ProjectBoardError {
    ProjectBoardError::LaunchFailed {
        reason: "no container runtime".into(),
        run: 1,
        run_state,
        released,
        restored: None,
        still_running: None,
    }
}

/// `store` with task `a` rewritten by `changed`, or run `run` added.
fn rewritten(store: &MemoryBoardStore, task: Option<TaskStatus>, run: Option<u32>) {
    let board = store.read().unwrap();
    let mut artifacts = vec![];
    if let Some(status) = task {
        let fields = TaskFields {
            status,
            ..board.tasks[0].fields().clone()
        };
        artifacts.push(BoardArtifact::Task(Task::new(fields).unwrap()));
    }
    if let Some(number) = run {
        let fields = RunFields {
            number,
            ..board.runs[0].fields().clone()
        };
        artifacts.push(BoardArtifact::Run(Run::new(fields).unwrap()));
    }
    store.write(&board.head, &artifacts, "rewritten").unwrap();
}

/// M1: a start that took its task from in progress (held, no run open)
/// is undone on the server as locally: folded back, the task released.
#[test]
fn an_undo_of_a_start_from_in_progress_folds_it_back() {
    let (store, undo) = begun(held_by_ada);
    assert_eq!(undo.from, TaskStatus::InProgress);
    let undone = served_as(&store, "Ada", "10:00:30").undone(&confirmed(1), &undo);
    assert_eq!(undone, launch_failed(RunAfterUndo::Folded, Some(true)));
    let (task, runs) = task_and_runs(&store);
    assert_eq!(
        (task.fields().status, runs[0].fields().outcome),
        (TaskStatus::Ready, Some(RunOutcome::Failed))
    );
}

/// M3: an undo while its run is open is refused unless the writer holds
/// the live claim, the task is in progress and the run is its latest; and
/// it must be of the start it is sent with. Each refusal writes nothing.
#[test]
fn an_undo_is_refused_by_each_of_its_rules() {
    let rule = ProjectBoardError::Rule;
    let (store, undo) = begun(ready);
    let refused = |who: &str, when: &str, start: &TaskStart| {
        let writes = store.writes();
        let undone = served_as(&store, who, when).undone(start, &undo);
        assert_eq!(store.writes(), writes, "{undone:?}: nothing written");
        undone
    };
    let now = "10:00:30";
    assert_eq!(
        refused("Bob", now, &confirmed(1)),
        rule(BoardRuleError::NotHolder)
    );
    assert_eq!(
        refused("Ada", "12:30:00", &confirmed(1)),
        rule(BoardRuleError::ClaimExpired)
    );
    let mismatch = refused("Ada", now, &confirmed(2));
    assert!(
        matches!(&mismatch, ProjectBoardError::Invalid(error) if error.field == "undo"),
        "{mismatch:?}"
    );
    rewritten(&store, None, Some(2));
    let not_latest = refused("Ada", now, &confirmed(1));
    assert!(
        matches!(
            not_latest,
            ProjectBoardError::Rule(BoardRuleError::RunsInconsistent(_))
        ),
        "{not_latest:?}"
    );
    let (store, undo) = begun(ready);
    rewritten(&store, Some(TaskStatus::Claimed), None);
    let undone = served_as(&store, "Ada", now).undone(&confirmed(1), &undo);
    assert_eq!(
        undone,
        rule(BoardRuleError::NotAllowed {
            from: TaskStatus::Claimed,
            to: TaskStatus::Ready,
        })
    );
    assert_eq!(
        task_and_runs(&store).1[0].fields().outcome,
        None,
        "still open"
    );
}

/// M3: a run another writer ended before the undo is found ended: the
/// task is left as that end moved it.
#[test]
fn an_undo_after_another_end_finds_it_ended_and_leaves_the_task() {
    let (store, undo) = begun(ready);
    let stopped = SwarmRunView {
        end: SwarmEnd::Cancelled,
        items: vec![],
        observed_tokens: 0,
    };
    let ada = Arc::new(store.client_as(person("Ada")));
    let folding = FoldBackRun::new(ada, SetClock::at("10:00:20"), Arc::new(MarkdownTextCells));
    folding.unwrap().execute(&slug("a"), 1, &stopped).unwrap();
    let (before, _) = task_and_runs(&store);
    let undone = served_as(&store, "Ada", "10:00:30").undone(&confirmed(1), &undo);
    assert_eq!(undone, launch_failed(RunAfterUndo::EndedAlready, None));
    assert_eq!(
        task_and_runs(&store).0,
        before,
        "the task as the end left it"
    );
}

//! S4c: a run no swarm will fold back is ended by hand: as stopped or
//! failed by its task's holder, as lost by whoever the domain lets.
use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::BoardOp;
use crate::application::project_board::fold_back::tests::plan;
use crate::application::project_board::ports::BoardWrite;
use crate::application::project_board::use_cases::ChangeBoardTask;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome, TaskStatus};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// A board as a start that wrote but never launched left it (the process
/// died between the two): task `board-store` held by Ada from 10:00 until
/// 12:00, run 1 going, and no swarm to fold it back.
fn stranded() -> (MemoryBoardStore, Task) {
    let store = board();
    let mut ready = fields("board-store");
    ready.accepted_plan = Some(1);
    let ready = Task::new(ready).unwrap();
    let accepted = plan();
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
        BoardArtifact::Task(task.clone()),
        BoardArtifact::Plan(accepted),
        BoardArtifact::Run(run),
    ];
    let written = store.write(&head, &artifacts, "board: board-store started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    (store, task)
}

fn ender(store: &MemoryBoardStore, who: &str, when: &str) -> EndRun {
    EndRun::new(
        Arc::new(store.client_as(person(who))),
        SetClock::at(when),
        Arc::new(MarkdownTextCells),
        None,
    )
    .unwrap()
}

fn ending(outcome: RunEndOutcome) -> RunEnd {
    RunEnd {
        task: slug("board-store"),
        run: 1,
        outcome,
        expected_updated: None,
    }
}

fn run_1(store: &MemoryBoardStore) -> Run {
    store.read().unwrap().runs[0].clone()
}

/// The holder ends the stranded run as stopped or failed: every item as
/// todo (no swarm report), its cost unknown; then the task can leave them.
/// A success is recorded the same way, so the task can go on to done.
#[test]
fn the_holder_ends_a_stranded_run_and_can_then_release_the_task() {
    for (outcome, want) in [
        (RunEndOutcome::Succeeded, RunOutcome::Succeeded),
        (RunEndOutcome::Stopped, RunOutcome::Stopped),
        (RunEndOutcome::Failed, RunOutcome::Failed),
    ] {
        let (store, _) = stranded();
        let mut facts = OpFacts::default();
        let ended = ender(&store, "Ada", "10:30:00")
            .execute_noted(&ending(outcome), Some(&mut facts))
            .unwrap()
            .run;
        assert_eq!(ended, run_1(&store));
        let fields = ended.fields();
        assert_eq!((fields.outcome, fields.cost), (Some(want), None));
        assert_eq!(fields.ended, Some(at("10:30:00")));
        let states: Vec<ItemState> = fields.items.iter().map(|item| item.state).collect();
        assert_eq!(states, [ItemState::Todo; 3], "every plan item, as todo");
        assert_eq!(
            (
                facts.run,
                facts.run_outcome,
                facts.attempts,
                facts.status_from
            ),
            (Some(1), Some(want), Some(1), Some(TaskStatus::InProgress))
        );
        assert!(facts.claim.is_some_and(|claim| claim.holder_is_writer));
        let release = ChangeBoardTask::new(
            Arc::new(store.client_as(person("Ada"))),
            SetClock::at("10:31:00"),
        )
        .unwrap();
        let task = release
            .execute(&slug("board-store"), &BoardOp::Move(TaskStatus::Ready))
            .unwrap();
        assert_eq!(task.fields().status, TaskStatus::Ready);
    }
}

/// Another person ends nothing while Ada's claim is live; once it expired
/// they take the task over and end its run as lost.
#[test]
fn another_ends_a_stranded_run_as_lost_once_they_hold_the_task() {
    let (store, _) = stranded();
    for outcome in RunEndOutcome::ALL {
        assert_eq!(
            ender(&store, "Bob", "11:00:00").execute(&ending(outcome)),
            Err(ProjectBoardError::Rule(BoardRuleError::NotHolder)),
            "{outcome:?}"
        );
    }
    assert_eq!(
        ender(&store, "Bob", "12:00:00").execute(&ending(RunEndOutcome::Lost)),
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolder)),
        "an expired claim is taken over first"
    );
    ChangeBoardTask::new(
        Arc::new(store.client_as(person("Bob"))),
        SetClock::at("12:00:00"),
    )
    .unwrap()
    .execute(&slug("board-store"), &BoardOp::TakeOver)
    .unwrap();
    let ended = ender(&store, "Bob", "12:01:00")
        .execute(&ending(RunEndOutcome::Lost))
        .unwrap()
        .run;
    assert_eq!(ended.fields().outcome, Some(RunOutcome::Lost));
    assert_eq!(run_1(&store), ended);
}

/// The holder's own claim expired: renew it first.
#[test]
fn a_holder_whose_claim_expired_renews_first() {
    let (store, _) = stranded();
    assert_eq!(
        ender(&store, "Ada", "12:00:00").execute(&ending(RunEndOutcome::Failed)),
        Err(ProjectBoardError::Rule(BoardRuleError::ClaimExpired))
    );
    assert_eq!(run_1(&store).fields().outcome, None);
}

/// A run ends once; one the task does not have is refused; and a pin on
/// the task as it was read refuses a task changed since.
#[test]
fn an_end_is_refused_for_a_run_ended_unknown_or_read_stale() {
    let (store, task) = stranded();
    let ada = ender(&store, "Ada", "10:30:00");
    let stale = RunEnd {
        expected_updated: Some(at("09:00:00")),
        ..ending(RunEndOutcome::Failed)
    };
    assert_eq!(
        ada.execute(&stale),
        Err(ProjectBoardError::Stale {
            id: slug("board-store"),
            expected: at("09:00:00"),
            updated: task.fields().updated.clone(),
        })
    );
    let unknown = RunEnd {
        run: 2,
        ..ending(RunEndOutcome::Failed)
    };
    assert_eq!(
        ada.execute(&unknown),
        Err(ProjectBoardError::Rule(BoardRuleError::NoSuchRun(2)))
    );
    let pinned = RunEnd {
        expected_updated: Some(task.fields().updated.clone()),
        ..ending(RunEndOutcome::Failed)
    };
    assert!(ada.execute(&pinned).is_ok());
    let writes = store.writes();
    assert_eq!(
        ada.execute(&ending(RunEndOutcome::Stopped)),
        Err(ProjectBoardError::Rule(BoardRuleError::RunEnded(1)))
    );
    assert_eq!(store.writes(), writes, "nothing written");
}

/// An outcome is one of the allowlist, by its own name.
#[test]
fn an_outcome_is_named_by_its_allowlist() {
    for outcome in RunEndOutcome::ALL {
        assert_eq!(RunEndOutcome::parse(outcome.as_str()), Some(outcome));
    }
    for other in ["Succeeded", "Lost", "", "running", "completed"] {
        assert_eq!(RunEndOutcome::parse(other), None, "{other}");
    }
}

/// Whether a run's swarm runs here, as told: coordinator `7f3a`.
struct Running(bool);

impl crate::application::project_board::ports::RunLiveness for Running {
    fn running_coordinator(
        &self,
        task: &crate::domain::project_board::value_objects::slug::Slug,
        run: u32,
    ) -> Option<String> {
        (self.0 && task.as_str() == "board-store" && run == 1).then(|| "7f3a".into())
    }
}

/// Review rounds 3-4 (M2, L3): a run whose swarm still runs where this
/// front end can see it is never ended by hand, by any outcome; nothing is
/// written, and the refusal names the coordinator to stop. One it does not
/// see running ends as ever, and says the front end checked.
#[test]
fn a_run_whose_swarm_still_runs_is_not_ended() {
    let (store, _) = stranded();
    let writes = store.writes();
    let watching = |running| {
        EndRun::new(
            Arc::new(store.client_as(person("Ada"))),
            SetClock::at("10:30:00"),
            Arc::new(MarkdownTextCells),
            Some(Arc::new(Running(running))),
        )
        .unwrap()
    };
    let live = ProjectBoardError::RunLive {
        task: slug("board-store"),
        run: 1,
        agent: "7f3a".into(),
    };
    for outcome in RunEndOutcome::ALL {
        let refused = watching(true).execute(&ending(outcome));
        assert_eq!(refused, Err(live.clone()), "{outcome:?}");
    }
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!(
        live.to_string(),
        "run 1 of board-store still has its swarm running in this session: stop its \
         coordinator first (agent_cmd kill 7f3a), then end the run"
    );
    let ended = watching(false)
        .execute(&ending(RunEndOutcome::Failed))
        .unwrap();
    assert_eq!(ended.liveness, LivenessCheck::Checked);
}

/// A front end that sees no swarm (the CLI) ends the run unchecked, and
/// says so.
#[test]
fn a_front_end_that_sees_no_swarm_ends_the_run_unchecked() {
    let (store, _) = stranded();
    let ended = ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Failed))
        .unwrap();
    assert_eq!(ended.liveness, LivenessCheck::NotChecked);
    assert_eq!(LivenessCheck::NotChecked.as_str(), "not_checked");
    assert_eq!(LivenessCheck::Checked.as_str(), "checked");
}

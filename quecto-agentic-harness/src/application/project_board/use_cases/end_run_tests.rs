//! S4c: a run is ended by hand: as succeeded, stopped or failed by its
//! task's holder, from its swarm's board as this front end kept it (or as
//! a swarm that reported nothing), as lost by whoever the domain lets.
use crate::application::project_board::dto::ReadPin;
use crate::domain::project_board::services::transitions::MoveReason;
use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::BoardOp;
use crate::application::project_board::fold_back::tests::{plan, swarm_item};
use crate::application::project_board::fold_back::{SwarmItemState, SwarmRunView};
use crate::application::project_board::ports::BoardWrite;
use crate::application::project_board::telemetry::ReportSeen;
use crate::application::project_board::use_cases::ChangeBoardTask;
use crate::application::project_board::use_cases::fixtures_tests::{
    Remote, SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::rule_error::{BoardRuleError, SwarmReport};
use crate::domain::project_board::value_objects::slug::Slug;
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
        Remote::holding_all(),
        None,
        None,
    )
    .unwrap()
}

/// The board run 1 of `board-store`'s swarm left as its coordinator
/// stopped, as a front end kept it (none for any other run), until it is
/// recorded.
struct Kept(std::sync::Mutex<Option<SwarmRunView>>);

impl SwarmRunReports for Kept {
    fn ended_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView> {
        match (task.as_str(), run) {
            ("board-store", 1) => self.0.lock().unwrap().clone(),
            _ => None,
        }
    }

    fn left_run(&self, _: &Slug, _: u32) -> Option<SwarmRunView> {
        None
    }

    fn recorded(&self, task: &Slug, run: u32) {
        assert_eq!((task.as_str(), run), ("board-store", 1), "the run kept");
        assert!(self.0.lock().unwrap().take().is_some(), "recorded once");
    }
}

/// Ada's end of the run, at 10:30, seeing no swarm running and reading
/// the board `kept` holds.
fn ender_keeping(store: &MemoryBoardStore, kept: Option<SwarmRunView>) -> EndRun {
    EndRun::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:30:00"),
        Arc::new(MarkdownTextCells),
        Remote::holding_all(),
        None,
        Some(Arc::new(Kept(std::sync::Mutex::new(kept)))),
    )
    .unwrap()
}

/// A swarm that ended as `end`, its items `api`, `store` and `docs` in
/// `states`, each with its evidence, held by `worker-1`, having spent
/// 52,000 tokens.
fn swarm(end: SwarmEnd, states: [SwarmItemState; 3]) -> SwarmRunView {
    let items = ["api", "store", "docs"]
        .iter()
        .zip(states)
        .map(|(id, state)| {
            swarm_item(
                &format!("{id}: Build the {id}"),
                state,
                Some("worker-1"),
                &[(&format!("docs/{id}.md"), "abc123")],
            )
        })
        .collect();
    SwarmRunView {
        end,
        items,
        observed_tokens: 52_000,
    }
}

fn ending(outcome: RunEndOutcome) -> RunEnd {
    RunEnd {
        task: slug("board-store"),
        run: 1,
        outcome,
        expected: None,
        stop: None,
    }
}

fn run_1(store: &MemoryBoardStore) -> Run {
    store.read().unwrap().runs[0].clone()
}

/// The holder ends the stranded run as stopped or failed: every item as
/// todo (no swarm report), its cost unknown; the end itself releases the
/// task back to ready (#2705 T5), so no move is left to make.
#[test]
fn the_holder_ends_a_stranded_run_and_the_end_releases_the_task() {
    for (outcome, want) in [
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
        assert_eq!(facts.status_to, Some(TaskStatus::Ready));
        let task = store.read().unwrap().tasks[0].clone();
        assert_eq!(
            (task.fields().status, task.fields().claim.clone()),
            (TaskStatus::Ready, None)
        );
        let again = ChangeBoardTask::new(
            Arc::new(store.client_as(person("Ada"))),
            SetClock::at("10:31:00"),
        )
        .unwrap()
        .execute(
            &slug("board-store"),
            &BoardOp::Move(TaskStatus::Ready, MoveReason::Absent),
        );
        assert!(again.is_err(), "nothing is left to release: {again:?}");
    }
}

/// The false success (light-well-tanking, run 1): the swarm did every
/// item, but nothing of it reached the board, so `end_run(succeeded)`
/// recorded a succeeded run with every task item todo, no owner, no
/// evidence and no cost. A success with nothing reported is refused, with
/// the items not done, and nothing is written; the run stays open, to be
/// ended as stopped.
#[test]
fn a_success_with_nothing_reported_is_refused_not_recorded_over_todo_items() {
    let (store, _) = stranded();
    let writes = store.writes();
    let undone = ProjectBoardError::Rule(BoardRuleError::ItemsNotDone {
        run: 1,
        items: vec![slug("api"), slug("store"), slug("docs")],
        report: SwarmReport::NoneKept,
    });
    assert_eq!(
        ender(&store, "Ada", "10:30:00").execute(&ending(RunEndOutcome::Succeeded)),
        Err(undone.clone()),
        "a front end that keeps no board (the CLI)"
    );
    assert_eq!(
        ender_keeping(&store, None).execute(&ending(RunEndOutcome::Succeeded)),
        Err(undone),
        "a front end that kept no board for this run"
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!(run_1(&store).fields().outcome, None, "the run stays open");
    let stopped = ender(&store, "Ada", "10:30:00")
        .execute(&ending(RunEndOutcome::Stopped))
        .unwrap();
    assert_eq!(stopped.run.fields().outcome, Some(RunOutcome::Stopped));
}

/// The fix: the board the swarm left as its coordinator stopped is kept,
/// and `end_run(succeeded)` records the run from it: every task item
/// done, with its owner and evidence, and the swarm's spend as its cost.
#[test]
fn a_success_is_recorded_from_the_board_its_swarm_left() {
    let (store, _) = stranded();
    let done = [SwarmItemState::Completed; 3];
    let ended = ender_keeping(&store, Some(swarm(SwarmEnd::Succeeded, done)))
        .execute(&ending(RunEndOutcome::Succeeded))
        .unwrap()
        .run;
    assert_eq!(ended, run_1(&store));
    let fields = ended.fields();
    assert_eq!(
        (fields.outcome, fields.cost),
        (Some(RunOutcome::Succeeded), Some(52_000))
    );
    for (item, id) in fields.items.iter().zip(["api", "store", "docs"]) {
        assert_eq!(item.id, slug(id));
        assert_eq!(item.state, ItemState::Done, "{id}");
        assert_eq!(item.owner.as_deref(), Some("worker-1"), "{id}");
        assert_eq!(item.evidence, [format!("docs/{id}.md at abc123")], "{id}");
    }
}

/// A board that shows an item short of done refuses the success, naming
/// it; a stop records what the board showed.
#[test]
fn a_success_is_refused_while_the_board_its_swarm_left_shows_an_item_undone() {
    let (store, _) = stranded();
    let short = [
        SwarmItemState::Completed,
        SwarmItemState::Submitted,
        SwarmItemState::Completed,
    ];
    let kept = swarm(SwarmEnd::Succeeded, short);
    assert_eq!(
        ender_keeping(&store, Some(kept.clone())).execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Rule(BoardRuleError::ItemsNotDone {
            run: 1,
            items: vec![slug("store")],
            report: SwarmReport::Kept,
        }))
    );
    let mut unseeded = kept.clone();
    unseeded.items.remove(1);
    unseeded
        .items
        .iter_mut()
        .for_each(|item| item.state = SwarmItemState::Completed);
    assert_eq!(
        ender_keeping(&store, Some(unseeded)).execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Rule(BoardRuleError::ItemsNotDone {
            run: 1,
            items: vec![slug("store")],
            report: SwarmReport::Kept,
        })),
        "an item the swarm never seeded is not done"
    );
    let stopped = ender_keeping(&store, Some(kept))
        .execute(&ending(RunEndOutcome::Stopped))
        .unwrap()
        .run;
    let fields = stopped.fields();
    assert_eq!(
        (fields.outcome, fields.cost),
        (Some(RunOutcome::Stopped), Some(52_000))
    );
    let states: Vec<ItemState> = fields.items.iter().map(|item| item.state).collect();
    assert_eq!(
        states,
        [ItemState::Done, ItemState::Review, ItemState::Done]
    );
}

/// A run past the board's bytes with its evidence is ended once more
/// without it, as a fold-back is; any other refusal is not retried.
#[test]
fn a_run_too_large_with_its_evidence_is_ended_without_it() {
    let (store, _) = stranded();
    store.fail_next_write(BoardStoreError::WriteTooLarge(
        "runs/ past its bytes".into(),
    ));
    let mut facts = OpFacts::default();
    let ended = ender_keeping(
        &store,
        Some(swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3])),
    )
    .execute_noted(&ending(RunEndOutcome::Succeeded), Some(&mut facts))
    .unwrap()
    .run;
    let fields = ended.fields();
    assert_eq!(fields.outcome, Some(RunOutcome::Succeeded));
    assert!(
        fields.items.iter().all(|item| item.evidence.is_empty()
            && item.state == ItemState::Done
            && item.owner.as_deref() == Some("worker-1")),
        "states and owners kept, evidence left out: {:?}",
        fields.items
    );
    assert_eq!(facts.attempts, Some(2), "both passes' attempts");
    let (store, _) = stranded();
    store.fail_next_write(BoardStoreError::InvalidWrite("no".into()));
    assert!(matches!(
        ender_keeping(
            &store,
            Some(swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3]))
        )
        .execute(&ending(RunEndOutcome::Succeeded)),
        Err(ProjectBoardError::Store(BoardStoreError::InvalidWrite(_)))
    ));
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
        Err(ProjectBoardError::Rule(BoardRuleError::NotHolderExpired)),
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
        expected: Some(ReadPin::Updated(at("09:00:00"))),
        stop: None,
        ..ending(RunEndOutcome::Failed)
    };
    assert_eq!(
        ada.execute(&stale),
        Err(ProjectBoardError::Stale {
            id: slug("board-store"),
            expected: ReadPin::Updated(at("09:00:00")),
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
        expected: Some(ReadPin::Updated(task.fields().updated.clone())),
        stop: None,
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

/// #2705 B9: an end pinned to a rev holds while the task is as it was at
/// that rev, whatever else the board took since, and is stale once the
/// task changed after it.
#[test]
fn an_end_pinned_by_rev_holds_until_the_task_changes() {
    let (store, _) = stranded();
    let read = store.read().unwrap().head;
    let ada = ender(&store, "Ada", "10:30:00");
    let pinned = |rev: &crate::application::project_board::ports::BoardHead| RunEnd {
        expected: Some(ReadPin::Rev(rev.clone())),
        stop: None,
        ..ending(RunEndOutcome::Failed)
    };
    let linked = ChangeBoardTask::new(
        Arc::new(store.client_as(person("Ada"))),
        SetClock::at("10:10:00"),
    )
    .unwrap()
    .execute(&slug("board-store"), &BoardOp::AddPr(9))
    .unwrap();
    let writes = store.writes();
    assert_eq!(
        ada.execute(&pinned(&read)),
        Err(ProjectBoardError::Stale {
            id: slug("board-store"),
            expected: ReadPin::Rev(read.clone()),
            updated: linked.fields().updated.clone(),
        })
    );
    assert_eq!(store.writes(), writes, "nothing written");
    let fresh = store.read().unwrap().head;
    assert!(ada.execute(&pinned(&fresh)).is_ok());
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
            Remote::holding_all(),
            Some(Arc::new(Running(running))),
            None,
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

/// #2663 and #2667: an end that folds the board its swarm left says so,
/// and notes that fold-back for its own `board_fold_back` record: how the
/// run ended, its items by state, what was left out and its cost.
#[test]
fn an_end_from_a_kept_board_notes_its_fold_back() {
    use crate::application::project_board::dto::FoldedBack;
    use crate::domain::project_board::value_objects::telemetry::{ItemCounts, RunOutcomeKind};
    let (store, _) = stranded();
    let mut facts = OpFacts::default();
    let done = [SwarmItemState::Completed; 3];
    let ended = ender_keeping(&store, Some(swarm(SwarmEnd::Succeeded, done)))
        .execute_noted(&ending(RunEndOutcome::Succeeded), Some(&mut facts))
        .unwrap()
        .run;
    assert_eq!(facts.swarm_report, Some(ReportSeen::Kept));
    let folded = facts.fold_back.expect("a kept board's fold-back is noted");
    assert_eq!((folded.task, folded.run), (slug("board-store"), 1));
    assert_eq!(folded.result, Ok(FoldedBack::Folded(ended.clone())));
    let noted = folded.facts;
    assert_eq!(noted.run_outcome, Some(RunOutcomeKind::Succeeded));
    assert_eq!(
        noted.items,
        Some(ItemCounts {
            todo: 0,
            doing: 0,
            review: 0,
            done: 3,
            blocked: 0
        })
    );
    assert_eq!(
        (
            noted.unmatched,
            noted.extra,
            noted.evidence,
            noted.evidence_dropped
        ),
        (Some(0), Some(0), Some(3), Some(0))
    );
    assert_eq!(
        (noted.cost_tokens, noted.attempts, noted.lost),
        (Some(52_000), Some(1), Some(0))
    );
    assert_eq!(
        noted.bytes,
        Some(u64::try_from(ended.fields().text_bytes()).unwrap())
    );
}

/// #2663: an end with no board kept says so and notes no fold-back, nor
/// does a loss, which folds nothing; an end past the board's bytes notes
/// both passes and the evidence it dropped.
#[test]
fn an_end_says_whether_a_board_was_kept() {
    let (store, _) = stranded();
    let mut facts = OpFacts::default();
    ender_keeping(&store, None)
        .execute_noted(&ending(RunEndOutcome::Stopped), Some(&mut facts))
        .unwrap();
    assert_eq!(facts.swarm_report, Some(ReportSeen::NoneKept));
    assert_eq!(facts.fold_back, None);
    // A front end with no keeper to ask (the CLI) checked nothing.
    let (store, _) = stranded();
    let mut unasked = OpFacts::default();
    ender(&store, "Ada", "10:30:00")
        .execute_noted(&ending(RunEndOutcome::Lost), Some(&mut unasked))
        .unwrap();
    assert_eq!(
        (unasked.swarm_report, unasked.fold_back),
        (Some(ReportSeen::NotChecked), None)
    );
    // A refused end says nothing of a board.
    let (store, _) = stranded();
    let mut refused = OpFacts::default();
    assert!(
        ender(&store, "Bob", "10:30:00")
            .execute_noted(&ending(RunEndOutcome::Stopped), Some(&mut refused))
            .is_err()
    );
    assert_eq!(refused.swarm_report, None);
    let (store, _) = stranded();
    store.fail_next_write(BoardStoreError::WriteTooLarge(
        "runs/ past its bytes".into(),
    ));
    let mut large = OpFacts::default();
    ender_keeping(
        &store,
        Some(swarm(SwarmEnd::Succeeded, [SwarmItemState::Completed; 3])),
    )
    .execute_noted(&ending(RunEndOutcome::Succeeded), Some(&mut large))
    .unwrap();
    let noted = large.fold_back.expect("noted").facts;
    assert_eq!(
        (noted.attempts, noted.evidence, noted.evidence_dropped),
        (Some(2), Some(0), Some(3))
    );
}

/// #2663 review round 3: a loss with a kept board folds its items onto
/// the run (#2657) and writes its own `board_fold_back` (#2667).
#[test]
fn a_lost_end_with_a_kept_board_notes_its_fold_back() {
    use crate::application::project_board::dto::FoldedBack;
    use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;
    let (store, _) = stranded();
    let mut facts = OpFacts::default();
    let done = [SwarmItemState::Completed; 3];
    let ended = ender_keeping(&store, Some(swarm(SwarmEnd::Succeeded, done)))
        .execute_noted(&ending(RunEndOutcome::Lost), Some(&mut facts))
        .unwrap()
        .run;
    assert_eq!(facts.swarm_report, Some(ReportSeen::Kept));
    let folded = facts.fold_back.expect("a lost end with a kept board folds");
    assert_eq!(folded.result, Ok(FoldedBack::Folded(ended)));
    assert_eq!(folded.facts.run_outcome, Some(RunOutcomeKind::Lost));
    assert_eq!(folded.facts.items.map(|items| items.done), Some(3));
}

#[path = "end_run_pushed_tests.rs"]
mod pushed;

#[path = "end_run_lost_tests.rs"]
mod lost;

#[path = "end_run_left_tests.rs"]
mod left;

#[path = "end_run_moves_tests.rs"]
mod moves;

#[path = "end_run_facts_tests.rs"]
mod facts;

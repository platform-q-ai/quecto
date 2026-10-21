use std::sync::Arc;

use super::*;
use crate::application::project_board::fold_back::tests::{plan, swarm_item};
use crate::application::project_board::fold_back::{SwarmEnd, SwarmItemState, SwarmItemView};
use crate::application::project_board::ports::{
    BoardArtifact, BoardStoreError, BoardWrite, FoldBackOutcome, FoldBackRecorder,
    ProjectBoardStore,
};
use crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, board, fields, person, slug,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::runs::end_lost;
use crate::domain::project_board::services::starts::start;
use crate::domain::project_board::value_objects::telemetry::{ItemCounts, RunOutcomeKind};
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// `run` of `task` ended as lost by Ada at 10:30, nothing reported.
fn lost(
    task: &Task,
    run: &Run,
) -> Result<Run, crate::domain::project_board::value_objects::rule_error::BoardRuleError> {
    let nothing = crate::domain::project_board::services::runs::Reported {
        items: vec![],
        cost: None,
        report: crate::domain::project_board::value_objects::rule_error::SwarmReport::NoneKept,
        unpushed: 0,
    };
    end_lost(
        task,
        (run, &plan()),
        nothing,
        &person("Ada"),
        &at("10:30:00"),
    )
}

/// A board holding task `board-store`, started by Ada at 10:00 from its
/// accepted plan: in progress, run 1 going.
fn started() -> (MemoryBoardStore, Task, Run) {
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
        BoardArtifact::Run(run.clone()),
    ];
    let written = store.write(&head, &artifacts, "board: board-store started");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    (store, task, run)
}

fn fold_back(store: &MemoryBoardStore) -> FoldBackRun {
    FoldBackRun::new(
        Arc::new(store.client()),
        SetClock::at("11:00:00"),
        Arc::new(MarkdownTextCells),
    )
    .unwrap()
}

fn ended(end: SwarmEnd, items: Vec<SwarmItemView>) -> SwarmRunView {
    SwarmRunView {
        end,
        items,
        observed_tokens: 52_000,
    }
}

fn swarm() -> SwarmRunView {
    ended(
        SwarmEnd::Succeeded,
        vec![
            swarm_item(
                "api: Build the api",
                SwarmItemState::Completed,
                Some("worker-1"),
                &[
                    ("https://github.com/o/r/pull/7", "4f2a9c1"),
                    ("costs $5", "4f2a9c1"),
                ],
            ),
            swarm_item(
                "store: Build the store",
                SwarmItemState::Submitted,
                Some("worker-2"),
                &[],
            ),
            swarm_item("lint: Lint it", SwarmItemState::Completed, None, &[]),
        ],
    )
}

fn run_on(store: &MemoryBoardStore) -> Run {
    let board = store.read().unwrap();
    board
        .runs
        .into_iter()
        .next()
        .expect("the board holds run 1")
}

#[test]
fn a_swarm_that_ended_folds_back_onto_its_run_alone_in_one_write() {
    let (store, task, _) = started();
    let writes = store.writes();
    let folded = fold_back(&store)
        .execute(&slug("board-store"), 1, &swarm())
        .unwrap();
    let FoldedBack::Folded(run) = folded else {
        panic!("folded back: {folded:?}");
    };
    assert_eq!(run, run_on(&store));
    assert_eq!(store.writes(), writes + 1);
    let fields = run.fields();
    assert_eq!(fields.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(fields.ended, Some(at("11:00:00")));
    assert_eq!(fields.cost, Some(52_000));
    let items: Vec<(&str, ItemState, Option<&str>, usize)> = fields
        .items
        .iter()
        .map(|item| {
            let owner = item.owner.as_deref();
            (item.id.as_str(), item.state, owner, item.evidence.len())
        })
        .collect();
    assert_eq!(
        items,
        [
            ("api", ItemState::Done, Some("worker-1"), 1),
            ("store", ItemState::Review, Some("worker-2"), 0),
            ("docs", ItemState::Todo, None, 0),
        ]
    );
    // The task is left as it was: its holder decides where it goes.
    assert_eq!(store.read().unwrap().tasks, [task]);
    let head = store.read().unwrap().head;
    assert_eq!(
        store.message_at(&head).as_deref(),
        Some("board: run 1 of board-store folded back, succeeded")
    );
}

#[test]
fn each_end_is_written_as_its_outcome() {
    for (end, outcome) in [
        (SwarmEnd::Failed, RunOutcome::Failed),
        (SwarmEnd::BudgetExhausted, RunOutcome::Failed),
        (SwarmEnd::Blocked, RunOutcome::Stopped),
        (SwarmEnd::Cancelled, RunOutcome::Stopped),
    ] {
        let (store, _, _) = started();
        let view = ended(end, vec![]);
        fold_back(&store)
            .execute(&slug("board-store"), 1, &view)
            .unwrap();
        assert_eq!(run_on(&store).fields().outcome, Some(outcome), "{end:?}");
    }
}

#[test]
fn a_run_folds_back_once_and_a_run_ended_by_a_merge_is_done_quietly() {
    let (store, _, _) = started();
    let use_case = fold_back(&store);
    let id = slug("board-store");
    use_case.execute(&id, 1, &swarm()).unwrap();
    let folded = run_on(&store);
    let writes = store.writes();
    let again = use_case.execute(&id, 1, &ended(SwarmEnd::Failed, vec![]));
    assert_eq!(again, Ok(FoldedBack::Ended(1)));
    assert_eq!((store.writes(), run_on(&store)), (writes, folded));

    // Completed by a merge: the run was ended as lost with its task.
    let (store, task, run) = started();
    let lost = lost(&task, &run).unwrap();
    let head = store.read().unwrap().head;
    store
        .write(&head, &[BoardArtifact::Run(lost.clone())], "board: merged")
        .unwrap();
    let writes = store.writes();
    let folded = fold_back(&store).execute(&id, 1, &swarm());
    assert_eq!(folded, Ok(FoldedBack::Ended(1)));
    assert_eq!((store.writes(), run_on(&store)), (writes, lost));
}

/// Another writer moves the board just before each of the next `times`
/// writes.
fn moved_under(store: &MemoryBoardStore, times: usize) {
    if times == 0 {
        return;
    }
    let other = store.client();
    store.before_next_write(move |board| {
        let snapshot = board.read().unwrap();
        let task = snapshot.tasks[0].clone();
        let written = other.write(&snapshot.head, &[BoardArtifact::Task(task)], "board: other");
        assert!(matches!(written, Ok(BoardWrite::Written(_))));
        moved_under(&other, times - 1);
    });
}

#[test]
fn a_contended_write_is_retried_on_a_fresh_board() {
    let (store, _, _) = started();
    moved_under(&store, 1);
    let mut facts = FoldBackFacts::default();
    let folded =
        fold_back(&store).execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts));
    assert!(matches!(folded, Ok(FoldedBack::Folded(_))), "{folded:?}");
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!(run_on(&store).fields().outcome, Some(RunOutcome::Succeeded));
}

#[test]
fn a_race_lost_to_another_fold_back_of_the_run_is_done_quietly() {
    let (store, task, run) = started();
    let other = store.client();
    store.before_next_write(move |board| {
        let snapshot = board.read().unwrap();
        let lost = lost(&task, &run).unwrap();
        let written = other.write(&snapshot.head, &[BoardArtifact::Run(lost)], "board: lost");
        assert!(matches!(written, Ok(BoardWrite::Written(_))));
    });
    let folded = fold_back(&store).execute(&slug("board-store"), 1, &swarm());
    assert_eq!(folded, Ok(FoldedBack::Ended(1)));
    assert_eq!(run_on(&store).fields().outcome, Some(RunOutcome::Lost));
}

#[test]
fn a_board_moved_under_every_attempt_is_contended() {
    let (store, _, _) = started();
    moved_under(
        &store,
        crate::application::project_board::use_cases::MAX_WRITE_ATTEMPTS,
    );
    let mut facts = FoldBackFacts::default();
    let folded =
        fold_back(&store).execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts));
    assert_eq!(folded, Err(ProjectBoardError::Contended { attempts: 3 }));
    assert!(run_on(&store).is_open());
    // A fold-back refused still tells its write's attempts.
    let attempts = Some(u64::try_from(MAX_WRITE_ATTEMPTS).unwrap());
    assert_eq!(
        facts,
        FoldBackFacts {
            attempts,
            lost: attempts,
            ..FoldBackFacts::default()
        }
    );
}

/// The board moved by another write of its task.
fn moved(board: &MemoryBoardStore) {
    let snapshot = board.read().unwrap();
    let task = snapshot.tasks[0].clone();
    let written = board.write(&snapshot.head, &[BoardArtifact::Task(task)], "board: other");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
}

fn too_large(why: &str) -> BoardStoreError {
    BoardStoreError::WriteTooLarge(why.into())
}

#[test]
fn a_run_the_store_finds_too_large_is_folded_again_without_its_evidence() {
    let (store, _, _) = started();
    // A lost race, the run too large; then a lost race again, and it lands.
    store.before_next_write(|board| {
        moved(board);
        board.before_next_write(|board| {
            board.fail_next_write(too_large("runs/ would hold too many bytes"));
            board.before_next_write(moved);
        });
    });
    let mut facts = FoldBackFacts::default();
    let folded = fold_back(&store)
        .execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts))
        .unwrap();
    let FoldedBack::Folded(run) = folded else {
        panic!("folded back: {folded:?}");
    };
    assert_eq!(run, run_on(&store));
    let fields = run.fields();
    assert_eq!(fields.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(fields.cost, Some(52_000));
    assert!(fields.items.iter().all(|item| item.evidence.is_empty()));
    assert_eq!(fields.items[0].owner.as_deref(), Some("worker-1"));
    assert_eq!(fields.items[0].state, ItemState::Done);
    assert_eq!((facts.evidence, facts.evidence_dropped), (Some(0), Some(2)));
    // Both passes' attempts and lost races.
    assert_eq!((facts.attempts, facts.lost), (Some(4), Some(2)));

    // Too large again without it: refused, nothing written.
    let (store, _, _) = started();
    let writes = store.writes();
    store.fail_next_write(too_large("past its bytes"));
    store.before_next_write(|board| {
        board.before_next_write(|board| board.fail_next_write(too_large("still past them")));
    });
    let mut facts = FoldBackFacts::default();
    let folded =
        fold_back(&store).execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts));
    assert_eq!(
        folded,
        Err(ProjectBoardError::Store(too_large("still past them")))
    );
    assert_eq!((store.writes(), facts.attempts), (writes, Some(2)));
    assert!(run_on(&store).is_open());
}

#[test]
fn any_other_invalid_write_is_refused_with_its_evidence_never_left_out() {
    let (store, _, _) = started();
    let writes = store.writes();
    let invalid = BoardStoreError::InvalidWrite("plan 1 of task board-store: line 2".into());
    store.fail_next_write(invalid.clone());
    let mut facts = FoldBackFacts::default();
    let folded =
        fold_back(&store).execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts));
    assert_eq!(folded, Err(ProjectBoardError::Store(invalid)));
    assert_eq!((store.writes(), facts.attempts), (writes, Some(1)));
    assert!(run_on(&store).is_open());
}

#[test]
#[should_panic(expected = "a board change writes as it declares")]
fn a_run_write_that_writes_its_task_too_is_a_bug() {
    let (store, task, run) = started();
    let clock = SetClock::at("11:00:00");
    let _ = super::super::board_change::commit_run(&store, &*clock, "board: x", None, |_, _| {
        Ok(vec![
            BoardArtifact::Task(task.clone()),
            BoardArtifact::Run(run.clone()),
        ])
    });
}

/// One outcome a recorder was given, owned.
type Outcome = (
    Slug,
    u32,
    Result<FoldedBack, ProjectBoardError>,
    FoldBackFacts,
    std::time::Duration,
);

/// The outcomes a recorder was given.
#[derive(Default)]
struct Recorded(std::sync::Mutex<Vec<Outcome>>);

impl FoldBackRecorder for Recorded {
    fn record(&self, outcome: FoldBackOutcome<'_>) {
        self.0.lock().unwrap().push((
            outcome.task.clone(),
            outcome.run,
            outcome.result.clone(),
            outcome.facts.clone(),
            outcome.elapsed,
        ));
    }
}

#[test]
fn a_recorded_fold_back_hands_its_outcome_facts_and_time_to_the_recorder() {
    let (store, _, _) = started();
    let clock = SetClock::at("11:00:00");
    *clock.tick.lock().unwrap() = std::time::Duration::from_millis(5);
    // The clock has run a while: the time taken is a difference, not a reading.
    *clock.elapsed.lock().unwrap() = std::time::Duration::from_secs(100);
    let use_case =
        FoldBackRun::new(Arc::new(store.client()), clock, Arc::new(MarkdownTextCells)).unwrap();
    let recorded = Recorded::default();
    let folded = use_case.execute_recorded(&slug("board-store"), 1, &swarm(), Some(&recorded));
    let taken = std::mem::take(&mut *recorded.0.lock().unwrap());
    let [(task, run, result, facts, elapsed)] = taken.as_slice() else {
        panic!("one outcome: {taken:?}");
    };
    assert_eq!((task, *run, result), (&slug("board-store"), 1, &folded));
    assert_eq!(facts.run_outcome, Some(RunOutcomeKind::Succeeded));
    // One tick: the start's reading, and the end's.
    assert_eq!(*elapsed, std::time::Duration::from_millis(5));
    // With no recorder, it is a plain fold-back: the run has ended now.
    let again = use_case.execute_recorded(&slug("board-store"), 1, &swarm(), None);
    assert_eq!(again, Ok(FoldedBack::Ended(1)));
    assert!(recorded.0.lock().unwrap().is_empty());
}

#[test]
fn a_run_task_or_plan_the_board_lacks_is_refused_and_nothing_written() {
    let (store, _, _) = started();
    let writes = store.writes();
    let use_case = fold_back(&store);
    assert_eq!(
        use_case.execute(&slug("board-store"), 2, &swarm()),
        Err(ProjectBoardError::Rule(BoardRuleError::NoSuchRun(2)))
    );
    assert_eq!(
        use_case.execute(&slug("elsewhere"), 1, &swarm()),
        Err(ProjectBoardError::NotFound(slug("elsewhere")))
    );
    store.refuse_next_write("the codec refused it");
    let mut facts = FoldBackFacts::default();
    assert_eq!(
        use_case.execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts)),
        Err(ProjectBoardError::Refused("the codec refused it".into()))
    );
    assert_eq!(
        facts,
        FoldBackFacts {
            attempts: Some(1),
            lost: Some(0),
            ..FoldBackFacts::default()
        }
    );
    assert_eq!(store.writes(), writes);
    assert!(run_on(&store).is_open());
}

#[test]
fn a_noted_fold_back_tells_kinds_and_counts_and_an_unnoted_one_nothing() {
    let (store, _, _) = started();
    let mut facts = FoldBackFacts::default();
    fold_back(&store)
        .execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut facts))
        .unwrap();
    let bytes = u64::try_from(run_on(&store).fields().text_bytes()).unwrap();
    assert_eq!(
        facts,
        FoldBackFacts {
            run_outcome: Some(RunOutcomeKind::Succeeded),
            items: Some(ItemCounts {
                todo: 1,
                doing: 0,
                review: 1,
                done: 1,
                blocked: 0
            }),
            unmatched: Some(1),
            extra: Some(1),
            evidence: Some(1),
            evidence_dropped: Some(1),
            cost_tokens: Some(52_000),
            attempts: Some(1),
            lost: Some(0),
            bytes: Some(bytes),
        }
    );
    // A run already ended notes only its write's attempts.
    let mut again = FoldBackFacts::default();
    fold_back(&store)
        .execute_noted(&slug("board-store"), 1, &swarm(), Some(&mut again))
        .unwrap();
    assert_eq!(
        again,
        FoldBackFacts {
            attempts: Some(1),
            lost: Some(0),
            ..FoldBackFacts::default()
        }
    );
}

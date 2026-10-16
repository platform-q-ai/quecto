//! `QueryBoard::summarized`: a listing's tasks each summarised from one
//! read of the board, only when that read is the board the listing was
//! read at, so a summary always matches its task's row.
use super::{QueryBoard, summarized_on};
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardHead, BoardSnapshot, BoardWrite, ProjectBoardStore,
};
use crate::application::project_board::task_summary::{
    PlanSummary, RowSummary, Summarized, TaskSummary,
};
use crate::application::project_board::use_cases::fixtures_tests::{board, fields, slug};
use crate::domain::project_board::entities::plan::{Plan, PlanFields};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::vocabulary::PlanState;
use crate::infrastructure::test_support::board_artifacts::{plan, run};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn task(id: &str) -> Task {
    Task::new(fields(id)).unwrap()
}

fn accepted(task: &str, version: u32) -> Plan {
    let base = plan(task, version).into_fields();
    Plan::new(PlanFields {
        state: PlanState::Accepted,
        accepted: Some((base.proposed_by.clone(), base.created.clone())),
        ..base
    })
    .unwrap()
}

/// Task `a` with an accepted plan and a run of it; task `b` with a
/// draft; task `c` with neither.
fn seeded() -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let artifacts = vec![
        BoardArtifact::Task(task("a")),
        BoardArtifact::Task(task("b")),
        BoardArtifact::Task(task("c")),
        BoardArtifact::Plan(accepted("a", 1)),
        BoardArtifact::Plan(plan("b", 1)),
        BoardArtifact::Run(run("a", 1)),
    ];
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store
}

/// Every task on `store`'s board, by id, as a listing reads it.
fn listing(store: &MemoryBoardStore) -> BoardListing {
    QueryBoard::new(Arc::new(store.client())).all().unwrap()
}

fn read(store: &MemoryBoardStore, listing: &BoardListing) -> Summarized {
    QueryBoard::new(Arc::new(store.client()))
        .summarized(listing)
        .unwrap()
}

/// Each listed task's summary, by id, when every one was read.
fn summarized(
    store: &MemoryBoardStore,
    listing: BoardListing,
) -> Option<Vec<(String, TaskSummary)>> {
    let Summarized::Rows(rows) = read(store, &listing) else {
        return None;
    };
    let ids = listing
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_string());
    let summaries = rows.into_iter().map(|row| match row {
        RowSummary::Summary(summary) => summary,
        RowSummary::Skipped(why) => panic!("a row skipped: {why}"),
    });
    Some(ids.zip(summaries).collect())
}

#[test]
fn each_listed_task_is_summarised_in_its_order() {
    let store = seeded();
    let entries = summarized(&store, listing(&store)).expect("read at the board's head");
    let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
    let plan = |entry: &(String, TaskSummary)| entry.1.plan;
    let version = |state| Some(PlanSummary { version: 1, state });
    assert_eq!(plan(&entries[0]), version(PlanState::Accepted));
    assert_eq!(plan(&entries[1]), version(PlanState::Draft));
    assert_eq!(plan(&entries[2]), None);
    let runs: Vec<Option<u32>> = entries
        .iter()
        .map(|(_, summary)| summary.run.as_ref().map(|run| run.number))
        .collect();
    assert_eq!(runs, [Some(1), None, None]);
    assert!(
        entries
            .iter()
            .all(|(_, summary)| !summary.may_be_incomplete)
    );
    let again = read(&store, &listing(&store));
    let rows = entries
        .into_iter()
        .map(|(_, summary)| RowSummary::Summary(summary))
        .collect();
    assert_eq!(again, Summarized::Rows(rows));
}

/// The listing's summaries are `show`'s judgement: a run file set aside
/// leaves only its own task's summary maybe incomplete.
#[test]
fn a_set_aside_run_file_is_named_on_its_tasks_summary() {
    let store = seeded();
    store.quarantine_key(ArtifactKey::Run(slug("a"), 2), "runs/a.2.md", "malformed");
    let entries = summarized(&store, listing(&store)).unwrap();
    let incomplete: Vec<bool> = entries
        .iter()
        .map(|(_, summary)| summary.may_be_incomplete)
        .collect();
    assert_eq!(incomplete, [true, false, false]);
    let shown = QueryBoard::new(Arc::new(store.client()))
        .show(&slug("a"))
        .unwrap();
    assert_eq!(shown.may_be_incomplete(), entries[0].1.may_be_incomplete);
}

/// A listing read before the board moved is not summarised from the
/// board it moved to: its rows could disagree with the summaries.
#[test]
fn a_listing_of_another_head_is_not_summarised() {
    let store = seeded();
    let stale = listing(&store);
    let head = store.read().unwrap().head;
    let written = store.write(&head, &[BoardArtifact::Task(task("d"))], "board: d");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    assert_eq!(read(&store, &stale), Summarized::Moved);
    assert!(summarized(&store, listing(&store)).is_some());
}

/// A listing at the board's head naming a task the board does not hold
/// as it lists it (a forged or broken index) disagrees with the board:
/// not summarised, and told apart from a board that moved.
#[test]
fn a_listing_whose_rows_are_not_the_boards_disagrees() {
    let store = seeded();
    let mut other = listing(&store);
    other.tasks[0] = Task::new(TaskFields {
        title: "Not the board's".into(),
        ..fields("a")
    })
    .unwrap();
    assert_eq!(read(&store, &other), Summarized::Disagrees);
    let mut missing = listing(&store);
    missing.tasks.push(task("z"));
    assert_eq!(read(&store, &missing), Summarized::Disagrees);
    store.quarantine("c", "malformed");
    let mut set_aside = listing(&store);
    set_aside.tasks.push(task("c"));
    assert_eq!(read(&store, &set_aside), Summarized::Disagrees);
}

/// A plan the board holds twice leaves its own task without a summary,
/// naming the duplicate as `show` refuses it; every other task is still
/// summarised.
#[test]
fn a_duplicate_plan_skips_its_own_tasks_summary_only() {
    let board = BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![task("a"), task("b")],
        plans: vec![plan("a", 1), plan("a", 1)],
        runs: Vec::<Run>::new(),
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined: vec![],
    };
    let listing = BoardListing {
        head: BoardHead("h".into()),
        tasks: vec![task("a"), task("b")],
        quarantined: vec![],
    };
    let Ok(Summarized::Rows(rows)) = summarized_on(&board, &listing) else {
        panic!("rows are read")
    };
    assert_eq!(
        rows[0],
        RowSummary::Skipped(Box::new(ProjectBoardError::Duplicate {
            key: ArtifactKey::Plan(slug("a"), 1)
        }))
    );
    assert!(
        matches!(&rows[1], RowSummary::Summary(summary) if summary.plan.is_none()),
        "{rows:?}"
    );
    assert_eq!(rows.len(), 2);
}

use serde_json::{Value, json};

use super::CompleteRun;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, StoredFile, accepted, member_row,
    running_board, stored_task,
};
use crate::application::swarm::dto::{CompleteRunRequest, RevalidateTaskRequest};
use crate::application::swarm::use_cases::RevalidateTask;
use crate::domain::swarm::{BoardError, RunState};

/// Python's `MemoryRepository`: one command criterion with accepted
/// evidence at R2, and one completed task whose evidence is at R1.
fn memory_repository() -> BoardState {
    let mut state = running_board(100.0);
    state.run.as_mut().unwrap().contract.criteria = json!([{"id": "test", "kind": "command"}]);
    state.evidence = vec![accepted("parent", "test", "R2", "command")];
    let mut task = stored_task(1, "completed", json!([]), Some("worker"));
    task.set("evidence", json!([{"artifact": "log", "revision": "R1"}]));
    state.tasks = vec![task];
    state
}

fn complete(actor: &str, revision: Value) -> CompleteRunRequest {
    CompleteRunRequest {
        actor: actor.to_owned(),
        revision,
    }
}

fn outcome(board: &MemoryBoard) -> (Option<RunState>, Option<String>) {
    let run = board.snapshot().run.unwrap().record;
    (run.status, run.outcome)
}

/// `test_completion_revalidation_and_transition_without_storage`: stale
/// task evidence refuses success until the coordinator revalidates the
/// task at the final revision; success then ends the run as a pause
/// holding `succeeded` (`completed`, `stop`, `paused`), and the
/// revalidation records the evidence it replaced.
#[test]
fn completion_revalidation_and_transition_without_storage() {
    let board = MemoryBoard::with(memory_repository());
    let clock = SteppingClock::fixed(50.0);
    let service = CompleteRun::new(board.clone(), clock.clone());
    let revalidate =
        RevalidateTask::new(board.clone(), clock, std::sync::Arc::new(CompactEncoding));
    assert_eq!(
        service
            .execute(complete("parent", json!("R2")))
            .unwrap_err(),
        BoardError::new("task evidence refers to stale revision")
    );
    revalidate
        .execute(RevalidateTaskRequest {
            actor: "parent".to_owned(),
            task_id: json!(1),
            revision: json!("R2"),
            evidence: json!([{"artifact": "rerun", "revision": "R2"}]),
        })
        .unwrap();
    service.execute(complete("parent", json!("R2"))).unwrap();
    assert_eq!(
        outcome(&board),
        (Some(RunState::PAUSED), Some("succeeded".to_owned()))
    );
    let events = board.snapshot().events;
    let actions: Vec<&str> = events.iter().map(|event| event.action.as_str()).collect();
    assert_eq!(
        actions[actions.len() - 3..],
        ["completed", "stop", "paused"]
    );
    let revalidated = &events[events.len() - 4];
    assert_eq!(revalidated.action, "revalidated");
    assert_eq!(
        revalidated.detail["previous_evidence"][0]["revision"],
        json!("R1")
    );
}

/// The three events carry Python's details: the revision as given, the
/// stop's `completed at {revision}` reason, and the clock's `started`.
#[test]
fn success_records_the_revision_and_the_clock() {
    let mut state = memory_repository();
    state.evidence = vec![accepted("parent", "test", "R1", "command")];
    let board = MemoryBoard::with(state);
    let service = CompleteRun::new(board.clone(), SteppingClock::fixed(42.5));
    service.execute(complete("parent", json!("R1"))).unwrap();
    let details: Vec<(String, Value)> = board
        .snapshot()
        .events
        .into_iter()
        .map(|event| (event.action, event.detail))
        .collect();
    assert_eq!(
        details,
        [
            ("completed".to_owned(), json!({"revision": "R1"})),
            (
                "stop".to_owned(),
                json!({"status": "succeeded", "reason": "completed at R1"})
            ),
            (
                "paused".to_owned(),
                json!({"reason": "completed at R1", "started": 42.5, "outcome": "succeeded"})
            ),
        ]
    );
    let run = board.snapshot().run.unwrap().record;
    assert_eq!(run.outcome_reason.as_deref(), Some("completed at R1"));
}

/// A refusal's text, the change of the board that causes it, and the
/// revision completed at.
type Refusal = (&'static str, fn(&mut BoardState), Value);

/// `test_completion_rejects_each_unsatisfied_requirement`, through the use
/// case: each refusal leaves the run running and records nothing.
#[test]
fn completion_rejects_each_unsatisfied_requirement() {
    let refusals: [Refusal; 6] = [
        ("completion revision required", |_| {}, json!(" ")),
        ("completion revision required", |_| {}, json!(2)),
        (
            "completion requires accepted evidence at the current revision for every criterion",
            |state| state.evidence.clear(),
            json!("R2"),
        ),
        (
            "completion requires accepted evidence at the current revision for every criterion",
            |state| state.evidence = vec![accepted("parent", "test", "R2", "review")],
            json!("R2"),
        ),
        (
            "settle outstanding work and file reservations before success",
            |state| {
                state.files.push(StoredFile {
                    path: "a".to_owned(),
                    task: 1,
                    owner: "worker".to_owned(),
                    claim: "c".to_owned(),
                    token: "t".to_owned(),
                });
            },
            json!("R2"),
        ),
        (
            "settle outstanding work and file reservations before success",
            |state| {
                state
                    .tasks
                    .push(stored_task(2, "submitted", json!([]), Some("worker")))
            },
            json!("R2"),
        ),
    ];
    for (message, change, revision) in refusals {
        let mut state = memory_repository();
        change(&mut state);
        let board = MemoryBoard::with(state);
        let service = CompleteRun::new(board.clone(), SteppingClock::fixed(50.0));
        assert_eq!(
            service.execute(complete("parent", revision)).unwrap_err(),
            BoardError::new(message)
        );
        assert_eq!(outcome(&board), (Some(RunState::RUNNING), None));
        assert!(board.snapshot().events.is_empty(), "{message}");
    }
}

/// Only the coordinator completes, and only a running run.
#[test]
fn only_the_coordinator_completes_a_running_run() {
    let mut state = memory_repository();
    state.members.push(member_row("worker", "live"));
    let board = MemoryBoard::with(state);
    let service = CompleteRun::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(complete("worker", json!("R2")))
            .unwrap_err(),
        BoardError::new("only the designated coordinator may do this")
    );
    let mut paused = memory_repository();
    paused.run.as_mut().unwrap().record.status = Some(RunState::PAUSED);
    let board = MemoryBoard::with(paused);
    let service = CompleteRun::new(board, SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(complete("parent", json!("R2")))
            .unwrap_err(),
        BoardError::new("run is paused; no new work permitted")
    );
}

/// Criteria only a file edited outside the board holds (an entry without
/// a text id, or with a kind that is neither `command` nor `review`) are
/// refused naming the record (`outside_edited_contract`); an evidence row
/// that is not text never satisfies a criterion, as in Python.
#[test]
fn edited_criteria_are_refused_and_edited_evidence_satisfies_nothing() {
    for criteria in [
        json!([{"kind": "command"}]),
        json!([{"id": "test", "kind": "other"}]),
        json!({}),
    ] {
        let mut state = memory_repository();
        state.run.as_mut().unwrap().contract.criteria = criteria;
        let board = MemoryBoard::with(state);
        let service = CompleteRun::new(board, SteppingClock::fixed(50.0));
        assert_eq!(
            service
                .execute(complete("parent", json!("R2")))
                .unwrap_err(),
            BoardError::new("the board's run criteria is not as the board writes it")
        );
    }
    let mut state = memory_repository();
    state.evidence[0].1.revision = json!(2);
    let board = MemoryBoard::with(state);
    let service = CompleteRun::new(board, SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(complete("parent", json!("R2")))
            .unwrap_err(),
        BoardError::new(
            "completion requires accepted evidence at the current revision for every criterion"
        )
    );
}

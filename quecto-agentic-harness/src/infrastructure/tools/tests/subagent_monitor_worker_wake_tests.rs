// A coordinator's workers report through its board (#2471): their plain
// turn ends reach the coordinator only as stranded work read from the board
// or the first good turn after a failure; a reply comes as `reply_ready`.

use super::*;
use crate::application::swarm::ports::WorkerBoardRead;
use crate::domain::swarm::RunStatus;
use crate::domain::swarm::worker_wake::WorkerBoard;
use crate::infrastructure::tools::subagent_registry::{
    NotificationRx, NotificationTx, SubagentEntry, SubagentNotification, SubagentRegistry,
    SwarmNoteState, new_notification_channel, new_registry,
};

/// A board whose answer the test can change between reads.
#[derive(Clone)]
struct Board(std::sync::Arc<std::sync::Mutex<Result<Option<WorkerBoard>, String>>>);

impl Board {
    fn answering(answer: Result<Option<WorkerBoard>, String>) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(answer)))
    }
    fn set(&self, answer: Result<Option<WorkerBoard>, String>) {
        *self.0.lock().unwrap() = answer;
    }
}

impl WorkerBoardRead for Board {
    fn worker_board(
        &self,
    ) -> crate::application::swarm::ports::PortFuture<
        '_,
        Result<Option<WorkerBoard>, crate::domain::error::DomainError>,
    > {
        let answer = self.0.lock().unwrap().clone();
        Box::pin(async move { answer.map_err(crate::domain::error::DomainError::Tool) })
    }
}

/// A running run with `ready` ready tasks and claims owned by `claimed_by`
/// (workers are `a` and `b`, their board members `m-a` and `m-b`).
fn work(ready: i64, claimed_by: &[&str]) -> Result<Option<WorkerBoard>, String> {
    Ok(Some(WorkerBoard {
        status: RunStatus::Running,
        ready,
        claimed_by: claimed_by
            .iter()
            .map(|owner| format!("m-{owner}"))
            .collect(),
        coordinator: "m-coordinator".into(),
    }))
}

/// A coordinator's registry holding `workers`, launched with `board`.
fn coordinator_with(workers: &[&str], board: &Board) -> SubagentRegistry {
    let registry = new_registry();
    for worker in workers {
        let mut entry = SubagentEntry::new(std::path::PathBuf::new(), 0);
        entry.coordinator_wake.worker.launcher_board = launcher_board_for(
            true,
            Some(format!("m-{worker}")),
            Some(std::sync::Arc::new(board.clone())),
        );
        registry.lock().unwrap().insert((*worker).to_owned(), entry);
    }
    registry
}

fn agent_start() -> serde_json::Value {
    serde_json::json!({"type": "agent_start"})
}

fn agent_end() -> serde_json::Value {
    serde_json::json!({"type": "agent_end", "messages": []})
}

fn agent_error() -> serde_json::Value {
    serde_json::json!({"type":"response","command":"agent_error","error":"quota exhausted"})
}

fn event(registry: &SubagentRegistry, tx: &NotificationTx, agent: &str, value: serde_json::Value) {
    apply_and_notify(registry, Some(tx), agent, &value);
}

/// Every note sent once spawned board reads have run.
async fn notes(rx: &mut NotificationRx) -> Vec<SubagentNotification> {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|note| note.notification)
        .collect()
}

fn stranded(agent: &str, claimed: i64, ready: i64) -> SubagentNotification {
    SubagentNotification::SwarmState {
        agent_id: agent.into(),
        state: SwarmNoteState::WorkersIdle { claimed, ready },
    }
}

fn ordinary(notes: &[SubagentNotification]) -> bool {
    matches!(notes, [SubagentNotification::Completed { .. }])
}

#[tokio::test]
async fn a_turn_end_with_every_claim_held_by_a_working_worker_wakes_nobody() {
    let board = Board::answering(work(0, &["b"]));
    let registry = coordinator_with(&["a", "b"], &board);
    let (tx, mut rx) = new_notification_channel();
    for worker in ["a", "b"] {
        event(&registry, &tx, worker, agent_start());
    }
    event(&registry, &tx, "a", agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn a_worker_that_stops_holding_its_claim_is_reported_while_another_works() {
    let board = Board::answering(work(0, &["a", "b"]));
    let registry = coordinator_with(&["a", "b"], &board);
    let (tx, mut rx) = new_notification_channel();
    for worker in ["a", "b"] {
        event(&registry, &tx, worker, agent_start());
    }
    event(&registry, &tx, "a", agent_end());
    let notes = notes(&mut rx).await;
    assert_eq!(notes, vec![stranded("a", 1, 0)]);
    assert_eq!(
        notes[0].to_message(),
        "Swarm work is stranded ('a' ended a turn last): 1 claimed task(s) held by workers not working, 0 ready task(s) with no worker working; check the board: swarm op=summary."
    );
}

#[tokio::test]
async fn stranded_work_is_reported_once_until_it_grows_or_its_worker_turns_again() {
    let board = Board::answering(work(0, &["a"]));
    let registry = coordinator_with(&["a", "b", "c"], &board);
    let (tx, mut rx) = new_notification_channel();
    for worker in ["a", "b", "c"] {
        event(&registry, &tx, worker, agent_start());
    }
    event(&registry, &tx, "a", agent_end());
    assert_eq!(notes(&mut rx).await, vec![stranded("a", 1, 0)]);
    event(&registry, &tx, "b", agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new(), "the same work, once");
    event(&registry, &tx, "b", agent_start());
    event(&registry, &tx, "b", agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new(), "b is not named");
    board.set(work(0, &["a", "b"]));
    event(&registry, &tx, "c", agent_end());
    assert_eq!(notes(&mut rx).await, vec![stranded("c", 2, 0)], "it grew");
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_end());
    assert_eq!(
        notes(&mut rx).await,
        vec![stranded("a", 2, 0)],
        "a named worker turned again and stopped holding its claim"
    );
}

#[tokio::test]
async fn a_worker_in_error_or_exited_is_not_working() {
    let board = Board::answering(work(0, &["b"]));
    let registry = coordinator_with(&["a", "b"], &board);
    let (tx, mut rx) = new_notification_channel();
    for worker in ["a", "b"] {
        event(&registry, &tx, worker, agent_start());
    }
    registry.lock().unwrap().get_mut("b").unwrap().status =
        crate::infrastructure::tools::subagent_registry::SubagentStatus::Exited;
    event(&registry, &tx, "a", agent_end());
    assert_eq!(notes(&mut rx).await, vec![stranded("a", 1, 0)]);
}

#[tokio::test]
async fn the_fallback_note_is_not_sent_once_a_newer_turn_has_begun() {
    let board = Board::answering(Err("database is locked".into()));
    let registry = coordinator_with(&["a"], &board);
    let (tx, mut rx) = new_notification_channel();
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_end());
    event(&registry, &tx, "a", agent_start());
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn ready_work_with_no_worker_working_is_reported() {
    let board = Board::answering(work(2, &[]));
    let registry = coordinator_with(&["a"], &board);
    let (tx, mut rx) = new_notification_channel();
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_end());
    assert_eq!(notes(&mut rx).await, vec![stranded("a", 0, 2)]);
}

#[tokio::test]
async fn a_board_that_cannot_be_read_or_names_another_coordinator_sends_the_ordinary_note() {
    for answer in [Err("database is locked".to_owned()), Ok(None)] {
        let registry = coordinator_with(&["a", "b"], &Board::answering(answer));
        let (tx, mut rx) = new_notification_channel();
        for worker in ["a", "b"] {
            event(&registry, &tx, worker, agent_start());
        }
        event(&registry, &tx, "a", agent_end());
        let notes = notes(&mut rx).await;
        assert!(ordinary(&notes), "{notes:?}");
    }
}

#[tokio::test]
async fn a_worker_s_reply_ready_is_sent_as_the_ordinary_note() {
    let board = Board::answering(work(0, &[]));
    let registry = coordinator_with(&["a", "b"], &board);
    let (tx, mut rx) = new_notification_channel();
    for worker in ["a", "b"] {
        event(&registry, &tx, worker, agent_start());
    }
    event(&registry, &tx, "a", agent_end());
    event(
        &registry,
        &tx,
        "a",
        serde_json::json!({"type": "reply_ready"}),
    );
    let reply = notes(&mut rx).await;
    assert!(ordinary(&reply), "{reply:?}");
}

#[tokio::test]
async fn a_reply_ready_from_a_child_that_is_not_a_worker_sends_nothing_extra() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "helper".into(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
    let (tx, mut rx) = new_notification_channel();
    event(
        &registry,
        &tx,
        "helper",
        serde_json::json!({"type": "reply_ready"}),
    );
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn the_first_good_turn_after_a_failure_is_sent_and_errors_still_are() {
    let board = Board::answering(work(0, &[]));
    let registry = coordinator_with(&["a"], &board);
    let (tx, mut rx) = new_notification_channel();
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_error());
    let failed = notes(&mut rx).await;
    assert!(
        matches!(failed[..], [SubagentNotification::Errored { .. }]),
        "{failed:?}"
    );
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_end());
    let recovered = notes(&mut rx).await;
    assert!(ordinary(&recovered), "{recovered:?}");
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn a_child_with_no_launcher_board_keeps_every_turn_end_note() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "helper".into(),
        SubagentEntry::new(std::path::PathBuf::new(), 0),
    );
    let (tx, mut rx) = new_notification_channel();
    for _ in 0..2 {
        event(&registry, &tx, "helper", agent_start());
        event(&registry, &tx, "helper", agent_end());
    }
    let notes = notes(&mut rx).await;
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes
            .iter()
            .all(|n| matches!(n, SubagentNotification::Completed { .. }))
    );
}

#[tokio::test]
async fn a_worker_turn_end_from_the_wire_is_settled_by_the_board() {
    let board = Board::answering(work(0, &["a"]));
    let registry = coordinator_with(&["a"], &board);
    let (tx, mut rx) = new_notification_channel();
    handle_monitor_line(
        r#"{"type":"agent_start"}"#,
        "a",
        &registry,
        Some(&tx),
        None,
        None,
    );
    handle_monitor_line(
        r#"{"type":"agent_end","messages":[]}"#,
        "a",
        &registry,
        Some(&tx),
        None,
        None,
    );
    assert_eq!(notes(&mut rx).await, vec![stranded("a", 1, 0)]);
}

#[test]
fn only_a_swarm_worker_of_the_run_s_creator_gets_a_launcher_board() {
    let read = || -> Option<std::sync::Arc<dyn WorkerBoardRead>> {
        Some(std::sync::Arc::new(Board::answering(Ok(None))))
    };
    let member = || Some("m-a".to_owned());
    assert!(launcher_board_for(true, member(), read()).is_some());
    assert!(launcher_board_for(false, member(), read()).is_none());
    assert!(launcher_board_for(true, None, read()).is_none());
    assert!(launcher_board_for(true, member(), None).is_none());
}

#[tokio::test]
async fn a_worker_s_task_reply_is_the_board_s_and_a_later_one_is_sent() {
    let board = Board::answering(work(0, &[]));
    let registry = coordinator_with(&["a"], &board);
    registry
        .lock()
        .unwrap()
        .get_mut("a")
        .unwrap()
        .coordinator_wake
        .worker
        .task_pending = true;
    let (tx, mut rx) = new_notification_channel();
    for _ in 0..2 {
        event(&registry, &tx, "a", agent_start());
        event(&registry, &tx, "a", agent_end());
        event(
            &registry,
            &tx,
            "a",
            serde_json::json!({"type": "reply_ready"}),
        );
    }
    let notes = notes(&mut rx).await;
    assert!(
        ordinary(&notes),
        "only the later instruction's reply: {notes:?}"
    );
}

#[tokio::test]
async fn a_reply_ready_after_a_failed_turn_leaves_the_error_as_the_reply() {
    let board = Board::answering(work(0, &[]));
    let registry = coordinator_with(&["a"], &board);
    let (tx, mut rx) = new_notification_channel();
    event(&registry, &tx, "a", agent_start());
    event(&registry, &tx, "a", agent_error());
    event(
        &registry,
        &tx,
        "a",
        serde_json::json!({"type": "reply_ready"}),
    );
    let notes = notes(&mut rx).await;
    assert!(
        matches!(notes[..], [SubagentNotification::Errored { .. }]),
        "{notes:?}"
    );
}

#[tokio::test]
async fn a_board_read_older_than_one_applied_is_stale() {
    let board = Board::answering(work(0, &["a"]));
    let registry = coordinator_with(&["a"], &board);
    registry.lock().unwrap().get_mut("a").unwrap().status =
        crate::infrastructure::tools::subagent_registry::SubagentStatus::Idle;
    let stale = WorkerBoard {
        status: RunStatus::Running,
        ready: 0,
        claimed_by: vec!["m-a".into()],
        coordinator: "m-coordinator".into(),
    };
    assert!(super::worker_wake::newly_stranded(&registry, &stale, 2).is_some());
    assert!(
        super::worker_wake::newly_stranded(&registry, &stale, 1).is_none(),
        "read 1 began before read 2, which is already applied"
    );
}

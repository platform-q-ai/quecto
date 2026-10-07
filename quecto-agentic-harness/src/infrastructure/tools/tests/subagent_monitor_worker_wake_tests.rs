// A coordinator's workers report through its board (#2471): their plain
// turn ends reach the coordinator only as a reply it is owed, or as one
// note when every worker is idle with work stranded on the board.

use super::*;
use crate::application::swarm::ports::SwarmRunControl;
use crate::domain::swarm::RunStatus;
use crate::domain::swarm::parent_wake::CoordinatorBoard;
use crate::infrastructure::tools::subagent_registry::{
    NotificationRx, SubagentEntry, SubagentNotification, SubagentRegistry, SwarmNoteState,
    new_notification_channel, new_registry,
};

/// A board that answers every read with its one answer.
struct Board(Result<Option<CoordinatorBoard>, String>);

impl SwarmRunControl for Board {
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: crate::domain::swarm::RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<crate::domain::swarm::RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async { Err(crate::domain::error::DomainError::Tool("unused".into())) })
    }
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        let answer = self.0.clone();
        Box::pin(async move { answer.map_err(crate::domain::error::DomainError::Tool) })
    }
}

fn running(claimed: i64, ready: i64) -> CoordinatorBoard {
    CoordinatorBoard {
        status: RunStatus::Running,
        outcome: None,
        ready,
        claimed,
        submitted: 0,
        idle_workers: 0,
    }
}

/// A coordinator's registry holding `workers`, each launched with a board
/// that answers `answer`.
fn coordinator_with(
    workers: &[&str],
    answer: Result<Option<CoordinatorBoard>, String>,
) -> SubagentRegistry {
    let registry = new_registry();
    for worker in workers {
        let mut entry = SubagentEntry::new(std::path::PathBuf::new(), 0);
        entry.coordinator_wake.launcher_board =
            Some(LauncherBoard(std::sync::Arc::new(Board(answer.clone()))));
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

/// Every note sent once spawned board reads have run.
async fn notes(rx: &mut NotificationRx) -> Vec<SubagentNotification> {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    std::iter::from_fn(|| rx.try_recv().ok())
        .map(|note| note.notification)
        .collect()
}

fn start_all(
    registry: &SubagentRegistry,
    tx: &crate::infrastructure::tools::subagent_registry::NotificationTx,
    workers: &[&str],
) {
    for worker in workers {
        apply_and_notify(registry, Some(tx), worker, &agent_start());
    }
}

#[tokio::test]
async fn a_worker_turn_end_while_another_works_wakes_nobody() {
    let registry = coordinator_with(&["a", "b"], Ok(Some(running(1, 0))));
    let (tx, mut rx) = new_notification_channel();
    start_all(&registry, &tx, &["a", "b"]);
    apply_and_notify(&registry, Some(&tx), "a", &agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn the_last_worker_idle_with_a_claimed_task_reports_the_stall_once() {
    let registry = coordinator_with(&["a", "b"], Ok(Some(running(1, 0))));
    let (tx, mut rx) = new_notification_channel();
    start_all(&registry, &tx, &["a", "b"]);
    apply_and_notify(&registry, Some(&tx), "a", &agent_end());
    apply_and_notify(&registry, Some(&tx), "b", &agent_end());
    let notes = notes(&mut rx).await;
    assert_eq!(
        notes,
        vec![SubagentNotification::SwarmState {
            agent_id: "b".into(),
            state: SwarmNoteState::WorkersIdle {
                claimed: 1,
                ready: 0
            },
        }]
    );
    assert_eq!(
        notes[0].to_message(),
        "Swarm workers are all idle ('b' ended a turn last) with 1 claimed and 0 ready task(s) unfinished; check the board: swarm op=summary."
    );
}

#[tokio::test]
async fn the_last_worker_idle_with_nothing_stranded_wakes_nobody() {
    let registry = coordinator_with(&["a"], Ok(Some(running(0, 0))));
    let (tx, mut rx) = new_notification_channel();
    start_all(&registry, &tx, &["a"]);
    apply_and_notify(&registry, Some(&tx), "a", &agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new());
}

#[tokio::test]
async fn a_board_that_cannot_be_read_or_names_another_coordinator_sends_the_ordinary_note() {
    for answer in [Err("database is locked".to_owned()), Ok(None)] {
        let registry = coordinator_with(&["a"], answer);
        let (tx, mut rx) = new_notification_channel();
        start_all(&registry, &tx, &["a"]);
        apply_and_notify(&registry, Some(&tx), "a", &agent_end());
        let notes = notes(&mut rx).await;
        assert!(
            matches!(notes[..], [SubagentNotification::Completed { .. }]),
            "{notes:?}"
        );
    }
}

#[tokio::test]
async fn a_reply_the_coordinator_is_owed_is_sent_once() {
    let registry = coordinator_with(&["a", "b"], Ok(Some(running(1, 0))));
    let (tx, mut rx) = new_notification_channel();
    start_all(&registry, &tx, &["a", "b"]);
    mark_reply_owed(&registry, "a");
    apply_and_notify(&registry, Some(&tx), "a", &agent_end());
    let reply = notes(&mut rx).await;
    assert!(
        matches!(reply[..], [SubagentNotification::Completed { .. }]),
        "{reply:?}"
    );
    apply_and_notify(&registry, Some(&tx), "a", &agent_start());
    apply_and_notify(&registry, Some(&tx), "a", &agent_end());
    assert_eq!(notes(&mut rx).await, Vec::new(), "owed once");
}

#[tokio::test]
async fn a_failed_worker_turn_still_reports_its_error() {
    let registry = coordinator_with(&["a"], Ok(Some(running(1, 0))));
    let (tx, mut rx) = new_notification_channel();
    start_all(&registry, &tx, &["a"]);
    apply_and_notify(
        &registry,
        Some(&tx),
        "a",
        &serde_json::json!({"type":"response","command":"agent_error","error":"boom"}),
    );
    let notes = notes(&mut rx).await;
    assert!(
        matches!(notes[..], [SubagentNotification::Errored { .. }]),
        "{notes:?}"
    );
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
        apply_and_notify(&registry, Some(&tx), "helper", &agent_start());
        apply_and_notify(&registry, Some(&tx), "helper", &agent_end());
    }
    let notes = notes(&mut rx).await;
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes
            .iter()
            .all(|n| matches!(n, SubagentNotification::Completed { .. }))
    );
}

#[test]
fn only_a_swarm_worker_of_the_container_s_creator_gets_a_launcher_board() {
    let board = || Some(LauncherBoard(std::sync::Arc::new(Board(Ok(None)))));
    assert!(launcher_board_for(true, true, board()).is_some());
    for (worker, creator) in [(true, false), (false, true), (false, false)] {
        assert!(
            launcher_board_for(worker, creator, board()).is_none(),
            "{worker} {creator}"
        );
    }
    assert!(launcher_board_for(true, true, None).is_none());
}

//! #2467: the run's coordinator reports `swarm_state` at every idle
//! boundary; nobody else does.
use super::super::super::uds_swarm_control::at_idle_boundary;
use super::super::dispatch_test_env::DispatchTestEnv as Env;
use crate::application::swarm::ports::SwarmRunControl;
use crate::domain::swarm::parent_wake::CoordinatorBoard;
use crate::domain::swarm::{RunControlAction, RunControlReceipt, RunStatus};

/// A board that answers `coordinator_board` with each of `answers` in turn.
struct Board(std::sync::Mutex<Vec<Result<Option<CoordinatorBoard>, String>>>);

impl SwarmRunControl for Board {
    fn nudge_watch(&self) {}
    fn apply(
        &self,
        _: RunControlAction,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<RunControlReceipt, crate::domain::error::DomainError>,
    > {
        Box::pin(async { Err(crate::domain::error::DomainError::Tool("no status".into())) })
    }
    fn coordinator_board(&self) -> crate::application::swarm::ports::CoordinatorBoardFuture<'_> {
        let next = self.0.lock().unwrap().remove(0);
        Box::pin(async move { next.map_err(crate::domain::error::DomainError::Tool) })
    }
}

fn running(claimed: i64) -> CoordinatorBoard {
    CoordinatorBoard {
        status: RunStatus::Running,
        outcome: None,
        ready: 0,
        claimed,
        submitted: 0,
        idle_workers: 0,
    }
}

/// The `swarm_state` lines one idle boundary per answer emits.
async fn boundaries(
    answers: Vec<Result<Option<CoordinatorBoard>, String>>,
) -> Vec<serde_json::Value> {
    let prompts = vec![false; answers.len()];
    boundaries_after(answers, prompts).await
}

/// [`boundaries`], with a client's prompt arriving before each boundary
/// `prompts` marks.
async fn boundaries_after(
    answers: Vec<Result<Option<CoordinatorBoard>, String>>,
    prompts: Vec<bool>,
) -> Vec<serde_json::Value> {
    boundaries_as(true, answers, prompts).await
}

/// [`boundaries_after`] for a process launched as a coordinator or not.
async fn boundaries_as(
    launched_coordinator: bool,
    answers: Vec<Result<Option<CoordinatorBoard>, String>>,
    prompts: Vec<bool>,
) -> Vec<serde_json::Value> {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(tx);
    let board = std::sync::Arc::new(Board(std::sync::Mutex::new(answers)));
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(board))
            .launched_as_coordinator(launched_coordinator),
    );
    for prompted in prompts {
        if prompted {
            ctx.turn_control
                .client_prompted
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        at_idle_boundary(&mut ctx).await;
    }
    let mut states = Vec::new();
    while let Ok(line) = rx.try_recv() {
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if value["type"] == "swarm_state" {
            states.push(value);
        }
    }
    states
}

#[tokio::test]
async fn the_coordinator_reports_its_wake_at_each_idle_boundary() {
    let states = boundaries(vec![Ok(Some(running(1))), Ok(Some(running(0)))]).await;
    assert_eq!(states.len(), 2, "{states:?}");
    assert_eq!(states[0]["wake"], "hold");
    assert_eq!(states[0]["status"], "running");
    assert_eq!(states[1]["wake"], "idle");
}

#[tokio::test]
async fn a_finished_run_is_reported_with_its_outcome() {
    let finished = CoordinatorBoard {
        status: RunStatus::Paused,
        outcome: Some(RunStatus::Succeeded),
        ..running(0)
    };
    let states = boundaries(vec![Ok(Some(finished))]).await;
    assert_eq!(states.len(), 1, "{states:?}");
    assert_eq!(states[0]["wake"], "finished");
    assert_eq!(states[0]["status"], "succeeded");
}

#[tokio::test]
async fn a_client_prompt_is_reported_once_at_the_next_boundary() {
    let states = boundaries_after(
        vec![
            Ok(Some(running(1))),
            Ok(Some(running(1))),
            Ok(Some(running(1))),
        ],
        vec![false, true, false],
    )
    .await;
    let prompted: Vec<_> = states
        .iter()
        .map(|state| state["prompted"].clone())
        .collect();
    assert_eq!(prompted, vec![false, true, false], "{states:?}");
}

#[tokio::test]
async fn a_member_that_is_not_the_coordinator_reports_nothing() {
    assert_eq!(
        boundaries(vec![Ok(None), Err("locked".into())]).await,
        Vec::<serde_json::Value>::new()
    );
}

#[tokio::test]
async fn a_process_not_launched_as_a_coordinator_never_reads_its_board() {
    let states = boundaries_as(false, vec![Ok(Some(running(0)))], vec![true]).await;
    assert_eq!(states, Vec::<serde_json::Value>::new());
}

#[tokio::test]
async fn a_paused_run_with_no_outcome_is_reported_as_paused() {
    let paused = CoordinatorBoard {
        status: RunStatus::Paused,
        ..running(1)
    };
    let states = boundaries(vec![Ok(Some(paused))]).await;
    assert_eq!(states[0]["wake"], "paused");
    assert_eq!(states[0]["status"], "paused");
}

#[tokio::test]
async fn an_unreadable_board_after_a_report_is_reported_as_unknown() {
    let states = boundaries(vec![Ok(Some(running(1))), Err("locked".into()), Ok(None)]).await;
    assert_eq!(states.len(), 3, "{states:?}");
    for state in &states[1..] {
        assert_eq!(state["wake"], "unknown");
        assert!(state.get("status").is_none(), "{state}");
    }
}

#[tokio::test]
async fn without_a_swarm_nothing_is_reported() {
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(tx);
    at_idle_boundary(&mut ctx).await;
    while let Ok(line) = rx.try_recv() {
        assert!(!line.contains("swarm_state"), "{line}");
    }
}

#[test]
fn swarm_state_is_sent_as_a_typed_event() {
    let event = super::super::super::protocol::AgentEvent::SwarmState {
        wake: crate::domain::swarm::parent_wake::WakeKind::Finished,
        status: Some("succeeded".into()),
        prompted: false,
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"type": "swarm_state", "wake": "finished", "status": "succeeded", "prompted": false})
    );
}

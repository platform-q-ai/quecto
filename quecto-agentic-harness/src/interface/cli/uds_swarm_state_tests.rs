//! #2467: the run's coordinator reports `swarm_state` at every idle
//! boundary; nobody else does.
use super::super::super::uds_swarm_control::at_idle_boundary;
use super::super::dispatch_test_env::DispatchTestEnv as Env;
use crate::application::swarm::ports::SwarmRunControl;
use crate::domain::swarm::parent_wake::CoordinatorBoard;
use crate::domain::swarm::{RunControlAction, RunControlReceipt};

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
    fn coordinator_board(
        &self,
    ) -> crate::application::subagent_launch::LaunchFuture<
        '_,
        Result<Option<CoordinatorBoard>, crate::domain::error::DomainError>,
    > {
        let next = self.0.lock().unwrap().remove(0);
        Box::pin(async move { next.map_err(crate::domain::error::DomainError::Tool) })
    }
}

fn running(claimed: i64) -> CoordinatorBoard {
    CoordinatorBoard {
        status: "running".into(),
        ready: 0,
        claimed,
        idle_workers: 0,
    }
}

/// The `swarm_state` lines one idle boundary per answer emits.
async fn boundaries(
    answers: Vec<Result<Option<CoordinatorBoard>, String>>,
) -> Vec<serde_json::Value> {
    let count = answers.len();
    let mut env = Env::with_unselected_workflow();
    let mut ctx = env.ctx();
    let (tx, mut rx) = tokio::sync::broadcast::channel(64);
    ctx.broadcast_tx = Some(tx);
    ctx.turn_control = std::sync::Arc::new(
        crate::interface::cli::uds_cancel::TurnControl::with_swarm_control(Some(
            std::sync::Arc::new(Board(std::sync::Mutex::new(answers))),
        )),
    );
    for _ in 0..count {
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
async fn a_finished_run_is_reported_with_its_status() {
    let finished = CoordinatorBoard {
        status: "complete".into(),
        ..running(0)
    };
    let states = boundaries(vec![Ok(Some(finished))]).await;
    assert_eq!(states.len(), 1, "{states:?}");
    assert_eq!(states[0]["wake"], "finished");
    assert_eq!(states[0]["status"], "complete");
}

#[tokio::test]
async fn a_member_that_is_not_the_coordinator_reports_nothing() {
    assert_eq!(
        boundaries(vec![Ok(None), Err("locked".into())]).await,
        Vec::<serde_json::Value>::new()
    );
}

#[tokio::test]
async fn an_unreadable_board_after_a_report_is_reported_as_unknown() {
    let states = boundaries(vec![Ok(Some(running(1))), Err("locked".into())]).await;
    assert_eq!(states.len(), 2, "{states:?}");
    assert_eq!(states[1]["wake"], "unknown");
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
        wake: super::super::super::protocol::SwarmWake::Finished,
        status: Some("complete".into()),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({"type": "swarm_state", "wake": "finished", "status": "complete"})
    );
}

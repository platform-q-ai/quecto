use super::*;
use crate::domain::swarm::RunStatus;
#[test]
fn delayed_pause_does_not_cancel_resumed_turn() {
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    let handle = Arc::new(std::sync::Mutex::new(CancelSlot::ScopedArmed(
        sender,
        RunStatus::Running,
        12,
    )));
    suspend_swarm_turn(&handle, 10, || true);
    assert_eq!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );
    suspend_swarm_turn(&handle, 13, || true);
    assert_eq!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    );
}
#[test]
fn delayed_terminal_settlement_preserves_fresh_report() {
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    let handle = Arc::new(std::sync::Mutex::new(CancelSlot::ScopedArmed(
        sender,
        RunStatus::Failed,
        10,
    )));
    suspend_swarm_turn(&handle, 10, || true);
    assert_eq!(
        receiver.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );
}
#[test]
fn pending_swarm_wakes_coalesce_to_highest_generation() {
    let control = TurnControl::default();
    assert!(control.queue_swarm_wake(10));
    for generation in [10, 12, 11, 12] {
        assert!(!control.queue_swarm_wake(generation));
    }
    assert_eq!(control.take_swarm_wake(10), 12);
    assert!(control.queue_swarm_wake(13));
    assert_eq!(control.take_swarm_wake(13), 13);
}

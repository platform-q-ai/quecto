use super::swarm_lifecycle::{Effects, snapshot};
use quecto::domain::swarm::RunStatus;
use std::sync::Mutex;
#[tokio::test]
async fn failed_turn_abort_cannot_skip_execution_cancellation_or_termination() {
    let mut state = snapshot();
    state.status = RunStatus::Failed;
    let effects = Effects(Mutex::new(vec![]));
    quecto::application::swarm::settle(&state, "parent", &effects)
        .await
        .unwrap();
    assert_eq!(*effects.0.lock().unwrap(), ["cancel", "abort", "terminate"]);
}

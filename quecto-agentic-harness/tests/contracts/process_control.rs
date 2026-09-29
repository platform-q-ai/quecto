use super::swarm_lifecycle::{Alive, Effects, snapshot};
use quecto::domain::swarm::RunStatus;
use std::sync::Mutex;
#[tokio::test]
async fn failed_turn_abort_cannot_skip_inference_suspension_or_termination() {
    let mut state = snapshot();
    state.status = RunStatus::Failed;
    state.coordinator = "parent".into();
    let effects = Effects(Mutex::new(vec![]));
    quecto::application::swarm::settle(&state, "parent", &effects, &Alive)
        .await
        .unwrap();
    assert_eq!(
        *effects.0.lock().unwrap(),
        ["suspend", "abort", "terminate"]
    );
}

#[tokio::test]
async fn terminal_coordinator_is_retained_after_local_inference_suspension() {
    let mut state = snapshot();
    state.status = RunStatus::Failed;
    let effects = Effects(Mutex::new(vec![]));
    quecto::application::swarm::settle(&state, "worker", &effects, &Alive)
        .await
        .unwrap();
    assert_eq!(*effects.0.lock().unwrap(), ["suspend"]);
}

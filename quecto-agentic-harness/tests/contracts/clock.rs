use super::swarm_lifecycle::{Time, snapshot};
use quecto::domain::swarm::RunStatus;
#[test]
fn injected_clock_enforces_exact_deadline_without_sleeping() {
    let state = snapshot();
    assert_eq!(
        quecto::application::swarm::observed_outcome(&state, &Time(99.)),
        RunStatus::Running
    );
    assert_eq!(
        quecto::application::swarm::observed_outcome(&state, &Time(100.)),
        RunStatus::BudgetExhausted
    );
}

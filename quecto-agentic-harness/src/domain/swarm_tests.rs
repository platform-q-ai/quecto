use super::*;

#[test]
fn only_the_bootstrap_placeholder_run_is_not_a_swarm() {
    assert!(!participates(RunStatus::Setup));
    for status in [
        RunStatus::Running,
        RunStatus::Paused,
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::Cancelled,
        RunStatus::BudgetExhausted,
    ] {
        assert!(participates(status), "{status:?}");
    }
}

#[test]
fn a_workflow_enabled_agent_cannot_create_a_swarm() {
    validate_swarm_creation(false).unwrap();
    let error = validate_swarm_creation(true).unwrap_err().to_string();
    assert!(error.contains("cannot create a swarm"), "{error}");
    assert!(error.contains("relaunch"), "{error}");
}

#[test]
fn workflow_is_refused_only_for_swarm_agents_that_request_it() {
    validate_workflow(false, true).unwrap();
    validate_workflow(true, false).unwrap();
    assert!(
        validate_workflow(true, true)
            .unwrap_err()
            .to_string()
            .contains("workflow is unavailable for swarm agents")
    );
}

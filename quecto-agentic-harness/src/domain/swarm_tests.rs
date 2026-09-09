use super::*;

#[test]
fn only_a_created_run_is_a_swarm() {
    // The bootstrap placeholder carries deadline 0 whatever happens to it
    // (setup, or failed by a reconcile that saw a member die).
    assert!(!participates(0.0));
    assert!(!participates(-1.0));
    // A created run has a future deadline and stays a swarm afterwards.
    assert!(participates(1.0));
    assert!(participates(1_800_000_000.0));
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

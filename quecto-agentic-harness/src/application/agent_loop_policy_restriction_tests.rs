use super::super::tests::*;
use super::RestrictedMockRegistry;
use crate::domain::tool::{ToolPolicyMutation, ToolPolicyMutationStatus};

#[test]
fn queued_policy_enable_preserves_restricted_status() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    {
        let mut policy = agent.tool_policy_state.lock().unwrap();
        policy.disabled_tools.insert("alpha".to_string());
    }
    agent.swap_registry(Box::new(RestrictedMockRegistry::new("alpha")));

    agent.queue_tool_policy_mutation(&[ToolPolicyMutation::enable("alpha", "try enable")]);
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued enable drains");

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert!(
        agent
            .tool_policy_state
            .lock()
            .unwrap()
            .disabled_tools
            .contains("alpha")
    );
}

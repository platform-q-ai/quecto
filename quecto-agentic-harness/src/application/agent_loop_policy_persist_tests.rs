use super::super::tests::*;
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolPolicyRequest,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

#[test]
fn immediate_persist_patch_request_reaches_registry_with_persist_flag() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::Parent,
        "durable parent preference",
    )]);
    request.persist = true;
    request.correlation_id = Some("persist-immediate".into());

    let reconciliation = agent
        .request_tool_policy(request, ToolPolicyApplyMode::ImmediateIfIdle)
        .expect("idle request applies immediately");

    assert_eq!(
        reconciliation.correlation_id.as_deref(),
        Some("persist-immediate")
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction,
        "mock registry returns this sentinel only when AgentLoop preserves persist:true"
    );
}

#[test]
fn queued_persist_patch_request_reaches_registry_with_persist_flag() {
    let (mut agent, _provider) = make_agent(vec![text_response("done")], vec![("alpha", "ok")]);
    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "alpha",
        ProfileAvailabilityScope::Parent,
        "durable parent preference",
    )]);
    request.persist = true;
    request.correlation_id = Some("persist-queued".into());

    assert!(
        agent
            .request_tool_policy(request, ToolPolicyApplyMode::AtNextTurnBoundary)
            .is_none()
    );
    let reconciliation = agent
        .drain_tool_policy_mutations_at_boundary()
        .expect("queued request drains");

    assert_eq!(
        reconciliation.correlation_id.as_deref(),
        Some("persist-queued")
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction,
        "mock registry returns this sentinel only when AgentLoop preserves persist:true"
    );
}

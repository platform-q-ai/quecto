use super::*;
use crate::application::ports::agent_gateway::{
    ToolPolicyApplyModePayload, ToolPolicyMutationPayload, ToolPolicyScopePayload,
};
use crate::application::use_cases::test_support::MockGateway;

#[tokio::test]
async fn forwards_valid_tool_policy_mutation() {
    let gateway = MockGateway::connected();
    let event = execute(
        &gateway,
        vec![ToolPolicyMutationPayload {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ToolPolicyScopePayload::Child,
            reason: Some("test".into()),
        }],
        ToolPolicyApplyModePayload::ImmediateIfIdle,
        ToolPolicyOperationPayload::Patch,
        None,
        false,
    )
    .await
    .expect("forwarded");
    assert!(matches!(
        event,
        crate::domain::event::AgentEvent::Response { .. }
    ));
    let sent = gateway.commands();
    assert!(matches!(
        sent.as_slice(),
        [AgentCommand::SetToolPolicy { .. }]
    ));
}

#[tokio::test]
async fn rejects_missing_identifier() {
    let err = execute(
        &MockGateway::connected(),
        vec![ToolPolicyMutationPayload {
            tool_id: None,
            name: None,
            scope: ToolPolicyScopePayload::Child,
            reason: None,
        }],
        ToolPolicyApplyModePayload::ImmediateIfIdle,
        ToolPolicyOperationPayload::Patch,
        None,
        false,
    )
    .await
    .expect_err("invalid");
    assert!(matches!(err, ApiError::InvalidRequest(_)));
}

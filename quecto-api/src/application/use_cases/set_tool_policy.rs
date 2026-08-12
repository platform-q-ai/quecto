use crate::application::ports::agent_gateway::{
    AgentCommand, AgentGateway, ToolPolicyApplyModePayload, ToolPolicyMutationPayload,
    ToolPolicyOperationPayload, ToolPolicyScopePayload,
};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Mutate catalogue-backed live tool policy through the agent command path.
pub async fn execute(
    gateway: &dyn AgentGateway,
    mutations: Vec<ToolPolicyMutationPayload>,
    mode: ToolPolicyApplyModePayload,
    operation: ToolPolicyOperationPayload,
    unlisted_scope: Option<ToolPolicyScopePayload>,
) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    if mutations.is_empty() && operation != ToolPolicyOperationPayload::Replace {
        return Err(ApiError::InvalidRequest(
            "at least one tool policy mutation is required".into(),
        ));
    }
    if operation == ToolPolicyOperationPayload::Replace && unlisted_scope.is_none() {
        return Err(ApiError::InvalidRequest(
            "replace tool policy requires unlistedScope".into(),
        ));
    }
    if mutations
        .iter()
        .any(|mutation| mutation.name.is_none() && mutation.tool_id.is_none())
    {
        return Err(ApiError::InvalidRequest(
            "each tool policy mutation requires name or toolId".into(),
        ));
    }
    gateway
        .send(AgentCommand::SetToolPolicy {
            mutations,
            mode,
            operation,
            unlisted_scope,
        })
        .await
}

#[cfg(test)]
#[path = "set_tool_policy_tests.rs"]
mod tests;

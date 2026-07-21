use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// List the agent's spawned subagents and their live status (#524).
pub async fn execute(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::GetSubagents).await
}

#[cfg(test)]
#[path = "get_subagents_tests.rs"]
mod tests;

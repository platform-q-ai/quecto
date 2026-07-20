use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Clear the agent's conversation history in-place without restarting it.
pub async fn execute(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::ClearHistory).await
}

#[cfg(test)]
#[path = "clear_history_tests.rs"]
mod tests;

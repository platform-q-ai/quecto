use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Queue `message` to be delivered when the agent finishes its current run.
pub async fn execute(gateway: &dyn AgentGateway, message: String) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    if message.is_empty() {
        return Err(ApiError::InvalidRequest("message must not be empty".into()));
    }
    gateway.send(AgentCommand::FollowUp { message }).await
}

#[cfg(test)]
#[path = "follow_up_tests.rs"]
mod tests;

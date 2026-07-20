use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Interrupt the running agent after its current tool and deliver `message`.
pub async fn execute(gateway: &dyn AgentGateway, message: String) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    if message.is_empty() {
        return Err(ApiError::InvalidRequest("message must not be empty".into()));
    }
    gateway.send(AgentCommand::Steer { message }).await
}

#[cfg(test)]
#[path = "steer_tests.rs"]
mod tests;

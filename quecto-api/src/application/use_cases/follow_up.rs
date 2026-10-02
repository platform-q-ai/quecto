use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Queue `message` to be delivered when the agent finishes its current run.
pub async fn execute(
    gateway: &dyn AgentGateway,
    message: String,
    images: Vec<quecto_image::ImagePayload>,
) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    let images = super::admit_message(&message, images)?;
    gateway
        .send(AgentCommand::FollowUp { message, images })
        .await
}

#[cfg(test)]
#[path = "follow_up_tests.rs"]
mod tests;

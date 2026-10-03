use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Interrupt the running agent after its current tool and deliver `message`.
pub async fn execute(
    gateway: &dyn AgentGateway,
    message: String,
    images: Vec<quecto_image::ImagePayload>,
) -> Result<AgentEvent, ApiError> {
    let images = super::admit_message(&message, images)?;
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::Steer { message, images }).await
}

#[cfg(test)]
#[path = "steer_tests.rs"]
mod tests;

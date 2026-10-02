use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

pub struct SendPromptInput {
    pub message: String,
    /// Images as the client sent them, not yet validated (#2422).
    pub images: Vec<quecto_image::ImagePayload>,
    pub streaming_behavior: Option<String>,
    pub wait_for_completion: bool,
}

pub async fn execute(
    gateway: &dyn AgentGateway,
    input: SendPromptInput,
) -> Result<AgentEvent, ApiError> {
    let images = super::admit_message(&input.message, input.images)?;
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    let command = AgentCommand::Prompt {
        message: input.message,
        images,
        streaming_behavior: input.streaming_behavior,
    };

    if input.wait_for_completion {
        gateway.send(command).await
    } else {
        gateway.enqueue(command).await
    }
}

#[cfg(test)]
#[path = "send_prompt_tests.rs"]
mod tests;

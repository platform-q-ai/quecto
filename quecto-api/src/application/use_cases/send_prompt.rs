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
    let images = admit(&input.message, input.images)?;
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

/// A prompt carries text, images, or both (#2422). Images are validated by
/// `quecto_image`, the same rules and refusal text as the agent's, so a
/// refused prompt is answered here and never sent.
fn admit(
    message: &str,
    images: Vec<quecto_image::ImagePayload>,
) -> Result<Vec<quecto_image::ImageAttachment>, ApiError> {
    let images = quecto_image::validate_images(images)
        .map_err(|refusal| ApiError::InvalidRequest(refusal.to_string()))?;
    if message.is_empty() && images.is_empty() {
        return Err(ApiError::InvalidRequest("message must not be empty".into()));
    }
    Ok(images)
}

#[cfg(test)]
#[path = "send_prompt_tests.rs"]
mod tests;

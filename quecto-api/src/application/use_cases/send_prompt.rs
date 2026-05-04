use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

pub struct SendPromptInput {
    pub message: String,
    pub streaming_behavior: Option<String>,
    pub wait_for_completion: bool,
}

pub async fn execute(
    gateway: &dyn AgentGateway,
    input: SendPromptInput,
) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    let command = AgentCommand::Prompt {
        message: input.message,
        streaming_behavior: input.streaming_behavior,
    };

    if input.wait_for_completion {
        gateway.send(command).await
    } else {
        gateway.enqueue(command).await
    }
}

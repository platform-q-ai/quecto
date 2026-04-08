use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

pub async fn execute(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::GetState).await
}

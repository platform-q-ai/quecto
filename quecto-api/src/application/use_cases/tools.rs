use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Return the agent's rich tool catalogue for control/query clients.
pub async fn catalogue<G: AgentGateway>(gateway: &G) -> Result<AgentEvent, ApiError> {
    gateway.send(AgentCommand::GetToolCatalogue).await
}

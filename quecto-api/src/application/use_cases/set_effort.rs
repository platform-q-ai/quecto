use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Effort levels accepted by the harness. The agent performs the authoritative
/// validation against the active model's provider vocabulary; we reject
/// obviously invalid values here so the API returns a deterministic 400 instead
/// of a round-trip failure.
const VALID_EFFORTS: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];

/// Set the reasoning effort applied to subsequent agent turns.
pub async fn execute(gateway: &dyn AgentGateway, effort: String) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    let effort = effort.trim().to_ascii_lowercase();
    if !VALID_EFFORTS.contains(&effort.as_str()) {
        return Err(ApiError::InvalidRequest(format!(
            "effort must be one of {}",
            VALID_EFFORTS.join(", ")
        )));
    }
    gateway.send(AgentCommand::SetEffort { effort }).await
}

#[cfg(test)]
#[path = "set_effort_tests.rs"]
mod tests;

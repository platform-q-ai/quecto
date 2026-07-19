use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// List the agent's spawned subagents and their live status (#524).
pub async fn execute(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::GetSubagents).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::use_cases::test_support::MockGateway;

    #[tokio::test]
    async fn forwards_get_subagents_when_connected() {
        let gw = MockGateway::connected();
        execute(&gw).await.unwrap();
        assert!(matches!(
            gw.commands().as_slice(),
            [AgentCommand::GetSubagents]
        ));
    }

    #[tokio::test]
    async fn rejects_when_disconnected() {
        let gw = MockGateway::disconnected();
        let err = execute(&gw).await.unwrap_err();
        assert!(matches!(err, ApiError::AgentNotConnected));
        assert!(gw.commands().is_empty());
    }
}

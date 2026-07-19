use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// Cancel the agent's current run.
pub async fn execute(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::Abort).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::use_cases::test_support::MockGateway;

    #[tokio::test]
    async fn forwards_abort_when_connected() {
        let gw = MockGateway::connected();
        execute(&gw).await.unwrap();
        assert!(matches!(gw.commands().as_slice(), [AgentCommand::Abort]));
    }

    #[tokio::test]
    async fn rejects_when_disconnected() {
        let gw = MockGateway::disconnected();
        let err = execute(&gw).await.unwrap_err();
        assert!(matches!(err, ApiError::AgentNotConnected));
        assert!(gw.commands().is_empty());
    }

    #[tokio::test]
    async fn propagates_transport_error() {
        let gw = MockGateway::failing(ApiError::Timeout(120));
        assert!(matches!(
            execute(&gw).await.unwrap_err(),
            ApiError::Timeout(120)
        ));
    }
}

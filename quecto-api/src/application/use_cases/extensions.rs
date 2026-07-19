use crate::application::ports::agent_gateway::{AgentCommand, AgentGateway};
use crate::domain::error::ApiError;
use crate::domain::event::AgentEvent;

/// List the agent's registered extensions.
pub async fn list(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::GetExtensions).await
}

/// Re-scan extension directories and reload script extensions.
pub async fn reload(gateway: &dyn AgentGateway) -> Result<AgentEvent, ApiError> {
    if !gateway.is_connected() {
        return Err(ApiError::AgentNotConnected);
    }
    gateway.send(AgentCommand::ReloadExtensions).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::use_cases::test_support::MockGateway;

    #[tokio::test]
    async fn list_forwards_get_extensions() {
        let gw = MockGateway::connected();
        list(&gw).await.unwrap();
        assert!(matches!(
            gw.commands().as_slice(),
            [AgentCommand::GetExtensions]
        ));
    }

    #[tokio::test]
    async fn reload_forwards_reload_extensions() {
        let gw = MockGateway::connected();
        reload(&gw).await.unwrap();
        assert!(matches!(
            gw.commands().as_slice(),
            [AgentCommand::ReloadExtensions]
        ));
    }

    #[tokio::test]
    async fn list_rejects_when_disconnected() {
        let err = list(&MockGateway::disconnected()).await.unwrap_err();
        assert!(matches!(err, ApiError::AgentNotConnected));
    }

    #[tokio::test]
    async fn reload_rejects_when_disconnected() {
        let err = reload(&MockGateway::disconnected()).await.unwrap_err();
        assert!(matches!(err, ApiError::AgentNotConnected));
    }
}

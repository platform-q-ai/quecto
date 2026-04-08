use crate::application::ports::agent_gateway::AgentGateway;

pub struct HealthCheckResult {
    pub healthy: bool,
    pub agent_connected: bool,
}

pub fn execute(gateway: &dyn AgentGateway) -> HealthCheckResult {
    let connected = gateway.is_connected();
    HealthCheckResult {
        healthy: connected,
        agent_connected: connected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::agent_gateway::{AgentCommand, EventSubscriber};
    use crate::domain::error::ApiError;
    use crate::domain::event::AgentEvent;
    use std::future::Future;
    use std::pin::Pin;

    struct MockGateway {
        connected: bool,
    }

    impl AgentGateway for MockGateway {
        fn send(
            &self,
            _cmd: AgentCommand,
        ) -> Pin<Box<dyn Future<Output = Result<AgentEvent, ApiError>> + Send + '_>> {
            Box::pin(async { Err(ApiError::AgentNotConnected) })
        }

        fn subscribe(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Box<dyn EventSubscriber>, ApiError>> + Send + '_>>
        {
            Box::pin(async { Err(ApiError::AgentNotConnected) })
        }

        fn is_connected(&self) -> bool {
            self.connected
        }
    }

    #[test]
    fn healthy_when_connected() {
        let gw = MockGateway { connected: true };
        let result = execute(&gw);
        assert!(result.healthy);
        assert!(result.agent_connected);
    }

    #[test]
    fn unhealthy_when_disconnected() {
        let gw = MockGateway { connected: false };
        let result = execute(&gw);
        assert!(!result.healthy);
        assert!(!result.agent_connected);
    }
}

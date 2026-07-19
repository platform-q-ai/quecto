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
    use crate::application::use_cases::test_support::MockGateway;

    #[test]
    fn healthy_when_connected() {
        let result = execute(&MockGateway::connected());
        assert!(result.healthy);
        assert!(result.agent_connected);
    }

    #[test]
    fn unhealthy_when_disconnected() {
        let result = execute(&MockGateway::disconnected());
        assert!(!result.healthy);
        assert!(!result.agent_connected);
    }
}

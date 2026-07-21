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
#[path = "health_check_tests.rs"]
mod tests;

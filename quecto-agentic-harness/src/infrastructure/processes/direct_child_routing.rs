//! [`DirectChildRouting`] over a direct child's UDS endpoint (#1935).
//!
//! One edge only: the identity is resolved against this harness's own
//! registry (uuid *and* launch generation must match a child this harness
//! launched), the `shutdown` / `terminate_delegated_agent` command is sent
//! as one correlated request, and the child's ACK is awaited within a bound.
//! The endpoint is whatever the launch captured — a direct socket path or the
//! parent-owned proxy bridge — so the protocol reaches container children
//! through the same transport as every other command.
use std::time::Duration;

use crate::application::subagents::ports::{ChildRoutingError, DirectChildRouting, PortFuture};
use crate::domain::subagent_teardown::{DelegatedAgentIdentity, RoutingDepth, ShutdownReason};
use crate::infrastructure::tools::subagent_registry::{
    SubagentRegistry, send_subagent_uds_command_with_timeout,
};

use super::owned_child_supervisor::ProtocolOutcome;

/// Bound on connect + request + ACK for one edge.
pub const PROTOCOL_ACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Send `shutdown` to the harness at `socket_path` and wait for its ACK.
/// Shared by the port adapter and by the supervisor's protocol attempts for
/// callers that only hold a socket path.
pub async fn shutdown_over_socket(
    socket_path: &std::path::Path,
    reason: ShutdownReason,
    timeout: Duration,
) -> Result<(), ChildRoutingError> {
    let command = serde_json::json!({"type": "shutdown", "reason": reason.as_str()});
    request_ack(socket_path, &command.to_string(), timeout).await
}

/// [`shutdown_over_socket`] in the supervisor's vocabulary.
pub async fn shutdown_protocol_attempt(
    socket_path: std::path::PathBuf,
    reason: ShutdownReason,
    timeout: Duration,
) -> ProtocolOutcome {
    match shutdown_over_socket(&socket_path, reason, timeout).await {
        Ok(()) => ProtocolOutcome::Acknowledged,
        Err(error) => ProtocolOutcome::Negative(error.to_string()),
    }
}

async fn request_ack(
    socket_path: &std::path::Path,
    command: &str,
    timeout: Duration,
) -> Result<(), ChildRoutingError> {
    if socket_path.as_os_str().is_empty() {
        return Err(ChildRoutingError::Unreachable("no endpoint".into()));
    }
    let response = send_subagent_uds_command_with_timeout(socket_path, command, timeout)
        .await
        .map_err(|e| ChildRoutingError::Unreachable(e.to_string()))?;
    let parsed: serde_json::Value = serde_json::from_str(response.trim())
        .map_err(|e| ChildRoutingError::Unreachable(format!("unparseable ack: {e}")))?;
    if parsed.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
        return Ok(());
    }
    let detail = parsed
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("command refused")
        .to_owned();
    Err(ChildRoutingError::Unreachable(detail))
}

/// The port adapter: resolves identities through the registry.
pub struct UdsDirectChildRouting {
    registry: SubagentRegistry,
    timeout: Duration,
}

impl UdsDirectChildRouting {
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            timeout: PROTOCOL_ACK_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Affirmative resolution: the uuid must name an entry this harness
    /// launched itself (it carries a launch generation) and the generation
    /// must match exactly. Merged descendants and restored rows carry no
    /// generation and are never direct children.
    fn endpoint(
        &self,
        child: &DelegatedAgentIdentity,
    ) -> Result<std::path::PathBuf, ChildRoutingError> {
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries
            .get(child.uuid.as_str())
            .ok_or(ChildRoutingError::NotADirectChild)?;
        match entry.launch_generation {
            Some(generation) if generation == child.generation => Ok(entry.socket_path.clone()),
            _ => Err(ChildRoutingError::NotADirectChild),
        }
    }
}

impl DirectChildRouting for UdsDirectChildRouting {
    fn shutdown_child<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        Box::pin(async move {
            let endpoint = self.endpoint(child)?;
            shutdown_over_socket(&endpoint, reason, self.timeout).await
        })
    }

    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        Box::pin(async move {
            let endpoint = self.endpoint(via)?;
            let command = serde_json::json!({
                "type": "terminate_delegated_agent",
                "target_uuid": target.uuid.as_str(),
                "target_generation": target.generation.get(),
                "remaining_depth": remaining_depth.hops(),
            });
            request_ack(&endpoint, &command.to_string(), self.timeout).await
        })
    }
}

#[cfg(test)]
#[path = "direct_child_routing_tests.rs"]
mod tests;

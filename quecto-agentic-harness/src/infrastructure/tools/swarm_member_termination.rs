//! Delegated termination of a swarm member (#1939).
//!
//! When a swarm run ends, the settling member (the coordinator, or a member
//! observing the terminal outcome) asks every other live member to stop.
//! Authority is the protocol and the locally owned handle, never a pid: a
//! member this harness launched itself is a delegated agent of its registry
//! and is ended through `KillDelegatedAgent` (claim → shutdown over its edge
//! → owned-handle fallback → compensation); any other member is reachable
//! only over the endpoint it registered with the store, and is asked to
//! shut down there. A member that is neither reachable by protocol nor
//! locally owned is reported failed, truthfully, and nothing is signalled.
//! `ProcessIdentity` stays what it is: the store's liveness observation.
use std::sync::Arc;

use crate::application::subagents::dto::{KillDelegatedAgentError, KillDelegatedAgentRequest};
use crate::application::subagents::use_cases::KillDelegatedAgent;
use crate::domain::error::DomainError;
use crate::domain::subagent_teardown::ShutdownReason;
use crate::domain::swarm::Member;
use crate::infrastructure::processes::direct_child_routing::{
    PROTOCOL_ACK_TIMEOUT, shutdown_over_socket,
};

use super::subagent_registry::SubagentRegistry;

/// Ask the member's harness to shut down over the endpoint it registered.
/// The only authority over a member this harness did not launch.
pub async fn shutdown_member_over_endpoint(member: &Member) -> Result<(), DomainError> {
    let Some(endpoint) = member.endpoint.as_deref().filter(|e| !e.is_empty()) else {
        return Err(DomainError::Tool(format!(
            "swarm member {} registered no endpoint and is not owned by this harness; not terminated",
            member.id
        )));
    };
    shutdown_over_socket(
        std::path::Path::new(endpoint),
        ShutdownReason::OperatorRequest,
        PROTOCOL_ACK_TIMEOUT,
    )
    .await
    .map_err(|error| {
        DomainError::Tool(format!(
            "swarm member {} did not accept shutdown over its endpoint ({error}) and is not owned by this harness; not terminated",
            member.id
        ))
    })
}

/// Termination over this harness's own delegated agents first, the
/// member's endpoint otherwise.
pub struct DelegatedSwarmMemberTermination {
    registry: SubagentRegistry,
    kill: Arc<KillDelegatedAgent>,
}

impl DelegatedSwarmMemberTermination {
    pub fn new(registry: SubagentRegistry, kill: Arc<KillDelegatedAgent>) -> Self {
        Self { registry, kill }
    }

    /// The uuid of the delegated agent this harness launched whose endpoint
    /// is the member's registered endpoint, if any. Affirmative: only a row
    /// carrying a launch generation (launched here) is a candidate — a
    /// merged descendant or restored row never is; whether the row is still
    /// live is the kill's own resolution to make (an exited row answers
    /// `already ended`, never a signal).
    fn launched_agent_for(&self, member: &Member) -> Option<String> {
        let endpoint = member.endpoint.as_deref().filter(|e| !e.is_empty())?;
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .filter(|(_, entry)| entry.launch_generation.is_some())
            .find(|(_, entry)| entry.socket_path.as_os_str() == endpoint)
            .map(|(_, entry)| entry.agent_uuid.as_str().to_owned())
    }

    pub async fn terminate(&self, member: &Member) -> Result<(), DomainError> {
        let Some(uuid) = self.launched_agent_for(member) else {
            return shutdown_member_over_endpoint(member).await;
        };
        match self
            .kill
            .execute(KillDelegatedAgentRequest { reference: uuid })
            .await
        {
            Ok(_) => Ok(()),
            // Its end is already owned by another path, or it already ended.
            Err(KillDelegatedAgentError::AlreadyStopping)
            | Err(KillDelegatedAgentError::Unresolved(_)) => Ok(()),
            Err(error) => Err(DomainError::Tool(format!(
                "swarm member {} could not be terminated through its delegated agent: {error}",
                member.id
            ))),
        }
    }
}

#[cfg(test)]
#[path = "swarm_member_termination_tests.rs"]
mod tests;

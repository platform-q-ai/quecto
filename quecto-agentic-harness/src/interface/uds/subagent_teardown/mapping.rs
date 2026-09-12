//! Wire ↔ application mapping for teardown commands. No policy lives here:
//! every rejection is a domain value-object refusal surfaced verbatim.
use crate::application::subagents::dto::{
    PrepareShutdownRequest, ShutdownTrigger, TerminateDelegatedAgentRequest,
};
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, RoutingDepth, ShutdownReason,
};

use super::wire::SubagentTeardownCommand;

/// A parsed command mapped onto exactly one application request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeardownRequest {
    Shutdown(PrepareShutdownRequest),
    TerminateDelegatedAgent(TerminateDelegatedAgentRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    UnknownReason(String),
    EmptyTargetUuid,
    InvalidDepth(String),
}

impl std::fmt::Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownReason(raw) => write!(f, "unknown shutdown reason {raw:?}"),
            Self::EmptyTargetUuid => f.write_str("target_uuid must not be empty"),
            Self::InvalidDepth(detail) => f.write_str(detail),
        }
    }
}

pub fn map_command(command: &SubagentTeardownCommand) -> Result<TeardownRequest, MappingError> {
    match command {
        SubagentTeardownCommand::Shutdown { reason, .. } => {
            let reason = ShutdownReason::parse(reason)
                .map_err(|unknown| MappingError::UnknownReason(unknown.0))?;
            Ok(TeardownRequest::Shutdown(PrepareShutdownRequest {
                reason,
                trigger: ShutdownTrigger::ProtocolCommand,
            }))
        }
        SubagentTeardownCommand::TerminateDelegatedAgent {
            target_uuid,
            target_generation,
            remaining_depth,
            ..
        } => {
            if target_uuid.trim().is_empty() {
                return Err(MappingError::EmptyTargetUuid);
            }
            let remaining_depth = RoutingDepth::new(*remaining_depth)
                .map_err(|error| MappingError::InvalidDepth(error.to_string()))?;
            Ok(TeardownRequest::TerminateDelegatedAgent(
                TerminateDelegatedAgentRequest {
                    target: DelegatedAgentIdentity::new(
                        target_uuid.as_str(),
                        LaunchGeneration::new(*target_generation),
                    ),
                    remaining_depth,
                },
            ))
        }
    }
}

#[cfg(test)]
#[path = "mapping_tests.rs"]
mod tests;

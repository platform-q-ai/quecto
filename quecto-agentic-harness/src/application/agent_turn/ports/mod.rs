//! Capability-local ports for running an agent turn.
//!
//! [`AgentLoop`] is the inbound port every delivery adapter (CLI, REPL, UDS)
//! drives to run a conversation through the model and the tools; the
//! application's `AgentLoopImpl` implements it. Moved out of the domain by
//! #1960: the domain keeps the pure turn vocabulary
//! (`AgentResult`, `AgentInfo`, `AgentProgressEvent`).
use std::future::Future;
use std::pin::Pin;

use crate::domain::agents::value_objects::agent::{AgentInfo, AgentResult};
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::shared::error::DomainError;

/// Port: the agent loop that processes messages through LLM + tools.
pub trait AgentLoop: Send + Sync {
    /// Process a conversation: send messages to the LLM, execute tool calls,
    /// and return the final assistant response with metadata.
    fn process<'a>(
        &'a mut self,
        messages: &'a mut Vec<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<AgentResult, DomainError>> + Send + 'a>>;

    /// Return information about this agent's configuration.
    fn info(&self) -> AgentInfo;
}

/// Inbound port for the inactive turn declaration adapter; identity/authority are trusted inputs.
pub trait DeclareTurnOutcome: Send + Sync {
    fn declare(
        &self,
        authority: crate::domain::turn_declaration::LaunchAuthority,
        identity: &crate::domain::turn_declaration::TurnIdentity,
        intent: crate::domain::turn_declaration::TurnIntent,
        reason: &str,
    ) -> Result<crate::domain::turn_declaration::TurnDeclaration, DomainError>;
}

/// Inbound port of the active declaration adapter (#2708): the identity of
/// the parent turn admitted now, if any, read when the tool runs.
pub trait BoundTurnDeclaration: DeclareTurnOutcome {
    fn capture(&self) -> Option<crate::domain::turn_declaration::TurnIdentity>;
}

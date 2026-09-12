//! Presents teardown results as correlated wire responses and owns the one
//! rule the application must never see: the shutdown ACK is fully written
//! and flushed before the execute token moves.
use std::future::Future;
use std::pin::Pin;

use crate::application::subagents::dto::{
    HarnessShutdownError, PreparedShutdown, TerminateDelegatedAgentError, TerminationRouted,
};

use super::wire::{
    SHUTDOWN_COMMAND, TERMINATE_DELEGATED_AGENT_COMMAND, TeardownResponse, TeardownResponseData,
};

/// A connection that can take one complete response. The adapter frames it
/// for its own wire mode (legacy newline JSON or length-prefixed frames) and
/// resolves only after the bytes are written *and* flushed to the peer; an
/// error means the peer may not have the frame.
pub trait AckWriter: Send + Sync {
    fn write_and_flush<'a>(
        &'a self,
        response: &'a TeardownResponse,
    ) -> Pin<Box<dyn Future<Output = Result<(), AckWriteError>> + Send + 'a>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckWriteError(pub String);

impl std::fmt::Display for AckWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ack write failed: {}", self.0)
    }
}

pub fn shutdown_ack(id: Option<&str>, prepared: &PreparedShutdown) -> TeardownResponse {
    TeardownResponse::ok(
        id,
        SHUTDOWN_COMMAND,
        TeardownResponseData::ShuttingDown {
            reason: prepared.reason.as_str().to_owned(),
        },
    )
}

pub fn shutdown_rejection(id: Option<&str>, error: &HarnessShutdownError) -> TeardownResponse {
    TeardownResponse::err(id, SHUTDOWN_COMMAND, error.to_string())
}

pub fn termination_response(id: Option<&str>, routed: &TerminationRouted) -> TeardownResponse {
    let data = match routed {
        TerminationRouted::ShutdownRequested { child } => TeardownResponseData::ShutdownRequested {
            child_uuid: child.uuid.as_str().to_owned(),
        },
        TerminationRouted::Forwarded {
            via,
            remaining_depth,
        } => TeardownResponseData::Forwarded {
            via_uuid: via.uuid.as_str().to_owned(),
            remaining_depth: remaining_depth.hops(),
        },
    };
    TeardownResponse::ok(id, TERMINATE_DELEGATED_AGENT_COMMAND, data)
}

pub fn termination_rejection(
    id: Option<&str>,
    error: &TerminateDelegatedAgentError,
) -> TeardownResponse {
    TeardownResponse::err(id, TERMINATE_DELEGATED_AGENT_COMMAND, error.to_string())
}

/// Generic rejection for a line that never reached a use case (framing,
/// mapping, authorization). `command` is the best-known command name.
pub fn edge_rejection(
    id: Option<&str>,
    command: &'static str,
    detail: impl Into<String>,
) -> TeardownResponse {
    TeardownResponse::err(id, command, detail)
}

/// Write and flush `response` as one frame; the caller decides what a
/// failure means (for a shutdown ACK: release, never execute).
pub async fn deliver(
    writer: &dyn AckWriter,
    response: &TeardownResponse,
) -> Result<(), AckWriteError> {
    writer.write_and_flush(response).await
}

#[cfg(test)]
#[path = "presenter_tests.rs"]
mod tests;

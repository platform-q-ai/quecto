//! Presents teardown results as correlated wire responses and owns the one
//! rule the application must never see: the shutdown ACK is fully written
//! and flushed before the execute token moves.
use std::future::Future;
use std::pin::Pin;

use crate::application::subagents::dto::{
    HarnessShutdownError, PreparedShutdown, TerminateDelegatedAgentError, TerminationResult,
    TerminationRouted,
};
use crate::application::subagents::ports::DownstreamRejection;
use crate::domain::subagent_teardown::TerminationRouteError;

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

/// The kind a `shutdown` refusal carries to the parent that asked: a
/// harness that already terminated is an ending target (`already_exited`,
/// its exit follows and the parent's ladder awaits it); any other refusal
/// is a plain rejection.
pub fn shutdown_rejection_kind(error: &HarnessShutdownError) -> &'static str {
    match error {
        HarnessShutdownError::AlreadyTerminated => DownstreamRejection::AlreadyExited.kind(),
        HarnessShutdownError::NotPrepared
        | HarnessShutdownError::UnknownToken
        | HarnessShutdownError::TokenReleased
        | HarnessShutdownError::LifecycleViolation(_)
        | HarnessShutdownError::ExecutionInterrupted => {
            DownstreamRejection::Rejected(String::new()).kind()
        }
    }
}

pub fn shutdown_rejection(id: Option<&str>, error: &HarnessShutdownError) -> TeardownResponse {
    TeardownResponse::err_of_kind(
        id,
        SHUTDOWN_COMMAND,
        shutdown_rejection_kind(error),
        error.to_string(),
    )
}

pub fn termination_response(id: Option<&str>, routed: &TerminationRouted) -> TeardownResponse {
    let result = |result: &Option<TerminationResult>| result.map(|r| r.as_str().to_owned());
    let data = match routed {
        TerminationRouted::ShutdownRequested { child, result: r } => {
            TeardownResponseData::ShutdownRequested {
                child_uuid: child.uuid.as_str().to_owned(),
                result: result(r),
            }
        }
        TerminationRouted::Forwarded {
            via,
            remaining_depth,
            result: r,
        } => TeardownResponseData::Forwarded {
            via_uuid: via.uuid.as_str().to_owned(),
            remaining_depth: remaining_depth.hops(),
            result: result(r),
        },
    };
    TeardownResponse::ok(id, TERMINATE_DELEGATED_AGENT_COMMAND, data)
}

/// The kind the hop above relays: a downstream refusal keeps the kind it
/// arrived with, so the root sees the owner's answer, not the relay's.
pub fn rejection_kind(error: &TerminateDelegatedAgentError) -> &'static str {
    match error {
        TerminateDelegatedAgentError::Rejected(TerminationRouteError::UnknownTarget(_)) => {
            DownstreamRejection::UnknownTarget.kind()
        }
        TerminateDelegatedAgentError::Rejected(TerminationRouteError::StaleGeneration {
            ..
        }) => DownstreamRejection::StaleGeneration.kind(),
        TerminateDelegatedAgentError::Rejected(_) => {
            DownstreamRejection::Rejected(String::new()).kind()
        }
        TerminateDelegatedAgentError::ChildUnreachable { .. } => {
            DownstreamRejection::Unreachable(String::new()).kind()
        }
        TerminateDelegatedAgentError::NotAccepting => DownstreamRejection::NotAccepting.kind(),
        TerminateDelegatedAgentError::TargetAlreadyExited(_) => {
            DownstreamRejection::AlreadyExited.kind()
        }
        TerminateDelegatedAgentError::TerminationFailed { .. } => {
            DownstreamRejection::Failed(String::new()).kind()
        }
        TerminateDelegatedAgentError::Downstream { rejection, .. } => rejection.kind(),
    }
}

pub fn termination_rejection(
    id: Option<&str>,
    error: &TerminateDelegatedAgentError,
) -> TeardownResponse {
    TeardownResponse::err_of_kind(
        id,
        TERMINATE_DELEGATED_AGENT_COMMAND,
        rejection_kind(error),
        error.to_string(),
    )
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

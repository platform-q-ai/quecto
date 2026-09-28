//! Capability-local effect ports of the external-agent capability (#2286).
//!
//! The session use case (S3) starts, and for the abort fallback restarts,
//! the agent through [`ExternalAgentLauncher`] and drives it through
//! [`ExternalAgentProcess`]. Signatures name only domain types and this
//! capability's DTOs: no process, pipe, runtime or wire vocabulary crosses
//! this boundary. Each port has a contract suite in
//! `tests/contracts/<port>.rs`, proven on the production adapter against a
//! mock agent program.

use std::future::Future;
use std::pin::Pin;

use super::dto::{
    ExternalAgentExit, ExternalAgentInputError, ExternalAgentLaunchError, ExternalAgentLaunchSpec,
};
use crate::domain::external_agent::stream::ExternalAgentEvent;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Starts one external agent process per call.
pub trait ExternalAgentLauncher: Send + Sync {
    /// Start the agent `spec` describes, isolated in its member directory.
    /// A missing agent program is [`ExternalAgentLaunchError::NotFound`].
    fn start<'a>(
        &'a self,
        spec: ExternalAgentLaunchSpec,
    ) -> PortFuture<'a, Result<Box<dyn ExternalAgentProcess>, ExternalAgentLaunchError>>;
}

/// One running external agent: turns in, events out. Dropping it closes
/// its input and has its process ended.
pub trait ExternalAgentProcess: Send + Sync {
    /// Write one user turn. Several turns go to one process.
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<(), ExternalAgentInputError>>;

    /// The next event of the agent's stream, in order; `None` once its
    /// output has ended. A line the adapter cannot decode is skipped and
    /// logged, never the end of the stream.
    fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>>;

    /// Close the agent's input: it finishes and exits with status 0.
    /// Idempotent; no turn can be written afterwards.
    fn close_input(&self) -> PortFuture<'_, ()>;

    /// Wait for the process to end.
    fn exited(&self) -> PortFuture<'_, ExternalAgentExit>;

    /// The last [`super::dto::EXTERNAL_AGENT_STDERR_TAIL_BYTES`] of its
    /// stderr, for diagnostics.
    fn stderr_tail(&self) -> String;
}

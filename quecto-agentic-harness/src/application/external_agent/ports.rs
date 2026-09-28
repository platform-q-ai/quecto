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
    SessionRecord,
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
    /// output has ended. A line the adapter cannot read (too long, not
    /// text) is an [`ExternalAgentEvent::LineSkipped`], one it cannot
    /// decode is logged; neither is the end of the stream.
    fn next_event(&self) -> PortFuture<'_, Option<ExternalAgentEvent>>;

    /// Close the agent's input: it finishes and exits with status 0.
    /// Idempotent; no turn can be written afterwards — a send not yet being
    /// written, even one already waiting, is answered
    /// [`ExternalAgentInputError::Closed`].
    fn close_input(&self) -> PortFuture<'_, ()>;

    /// Wait for the process to end. Consumes no output: events unread
    /// when it ends stay readable through [`Self::next_event`], so a
    /// session may wait on both at once and lose none. An agent writes
    /// only as fast as its output is read, so a caller awaiting this alone
    /// while nobody reads can wait forever: one that has stopped reading
    /// waits with [`Self::exited_discarding_output`]. Returns once the
    /// agent's diagnostics are whole too, or within a short bound, so
    /// [`Self::stderr_tail`] carries its last words.
    fn exited(&self) -> PortFuture<'_, ExternalAgentExit>;

    /// [`Self::exited`], reading and discarding the output no one has read
    /// meanwhile, so an agent whose output no caller reads still finishes
    /// and exits. The events it discards are lost.
    fn exited_discarding_output(&self) -> PortFuture<'_, ExternalAgentExit>;

    /// The last [`super::dto::EXTERNAL_AGENT_STDERR_TAIL_BYTES`] of its
    /// stderr, for diagnostics.
    fn stderr_tail(&self) -> String;
}

/// Where a member session records each of its decisions and effects
/// (#2287): a [`SessionRecord`] carries ids, kinds, sizes and durations
/// only. Recording never fails the session and never blocks it.
pub trait ExternalAgentTelemetry: Send + Sync {
    fn record(&self, record: &SessionRecord);
}

//! Capability-local effect ports of the external-agent capability (#2286).
//!
//! The session use case (#2287) starts the agent through
//! [`ExternalAgentLauncher`], drives it through [`ExternalAgentProcess`],
//! records its decisions through [`ExternalAgentTelemetry`], times its
//! waits through [`ExternalAgentClock`] and hands the work that must outlive
//! its caller (an ended member's exit, #2304) to [`ExternalAgentSpawner`]. Signatures name only domain types and this
//! capability's DTOs: no process, pipe, runtime or wire vocabulary crosses
//! this boundary. Each port has a contract suite in
//! `tests/contracts/<port>.rs`, proven on the production adapter against a
//! mock agent program.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::dto::{
    AgentClockInstant, ExternalAgentExit, ExternalAgentInputError, ExternalAgentLaunchError,
    ExternalAgentLaunchSpec, SessionRecord, UserTurnId,
};
use crate::domain::external_agent::stream::ExternalAgentEvent;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A user turn [`ExternalAgentProcess::queue_user_turn`] queued.
pub struct QueuedUserTurn<'a> {
    /// The id the agent's results name it by.
    pub id: UserTurnId,
    /// Resolves once it is written, or failed to be (the input closed or
    /// its writer failed first). Dropping it does not withdraw the turn.
    pub written: PortFuture<'a, Result<(), ExternalAgentInputError>>,
}

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
    /// Queue one user turn for writing. Several turns go to one process,
    /// also while one runs: the agent folds a turn written mid-turn into
    /// the running one, or runs it as its own turn once that ends.
    ///
    /// Cancel-safe in two steps (#2287 review 5): dropped before it
    /// answers, nothing is queued; once it answers, the turn is queued
    /// under the id answered, by which the agent's results name it
    /// ([`crate::domain::external_agent::stream::ResultEvent::user_turn_ids`]),
    /// and is written whole unless the input closes or fails first,
    /// whether or not [`QueuedUserTurn::written`] is awaited.
    fn queue_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<QueuedUserTurn<'a>, ExternalAgentInputError>>;

    /// Queue one user turn and wait until it is written; answers its id
    /// (see [`Self::queue_user_turn`]).
    fn send_user_turn<'a>(
        &'a self,
        text: &'a str,
    ) -> PortFuture<'a, Result<UserTurnId, ExternalAgentInputError>> {
        Box::pin(async move {
            let queued = self.queue_user_turn(text).await?;
            queued.written.await.map(|()| queued.id)
        })
    }

    /// Ask the agent to stop its running turn and withdraw every user turn
    /// still queued. It answers with an
    /// [`ExternalAgentEvent::InterruptAnswered`]; a stopped turn still
    /// ends with its one `result`. Idle, nothing is stopped and no
    /// `result` follows. Written in order with the user turns.
    fn interrupt(&self) -> PortFuture<'_, Result<(), ExternalAgentInputError>>;

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

    /// Once the member's records are done (#2304): wait, for at most the
    /// adapter's own bound and never blocking a thread, until what was
    /// recorded is kept. A record made afterwards may be lost. Dropping an
    /// adapter never waits: this is where its records are made safe.
    fn finish(&self) -> PortFuture<'_, ()>;
}

/// The session's time (#2287): how long it waits on a quiet stream. The
/// adapter picks the scale; the session only compares its own instants.
pub trait ExternalAgentClock: Send + Sync {
    /// Now, on the adapter's monotonic scale.
    fn now(&self) -> AgentClockInstant;

    /// Resolves once `duration` has passed on the same scale.
    fn sleep(&self, duration: Duration) -> PortFuture<'_, ()>;
}

/// Work handed off to run on its own: it owns everything it uses.
pub type DetachedWork = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Runs work apart from the caller's future (#2304): an ended member's
/// exit is waited for and recorded however long it takes (at most
/// [`super::dto::EXIT_GRACE`]), while the session answers its callers, and
/// even when the caller that ended it gives up.
pub trait ExternalAgentSpawner: Send + Sync {
    /// Run `work` to its end, whether or not the caller's future survives.
    fn spawn(&self, work: DetachedWork);
}

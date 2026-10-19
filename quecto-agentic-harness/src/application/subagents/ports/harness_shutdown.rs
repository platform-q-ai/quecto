//! The self-shutdown transaction's ports: lifecycle and lineage, turn
//! cancellation, the owner's exit announcement, retained environments, session
//! persistence, the clock, exit readiness and the detached run.

use super::PortFuture;
use crate::domain::agents::services::subagent_teardown::HarnessLifecycleState;
use crate::domain::agents::services::subagent_teardown::LineageSnapshot;
use crate::domain::agents::services::subagent_teardown::ShutdownReason;
use std::future::Future;
use std::pin::Pin;

/// This harness's lifecycle state and its known delegation lineage.
pub trait SubagentLifecycleRepository: Send + Sync {
    fn lifecycle(&self) -> HarnessLifecycleState;
    /// Store a state the domain has already validated as a legal transition.
    fn set_lifecycle(&self, state: HarnessLifecycleState);
    fn lineage(&self) -> LineageSnapshot;
}

/// Cancels whatever turn is in flight so shutdown never waits on a provider.
pub trait TurnCancellation: Send + Sync {
    /// Returns `true` when a turn was actually interrupted.
    fn cancel_in_flight_turn(&self) -> PortFuture<'_, bool>;
}

/// The owner's exit announcement (#2070). The TUI that owns this harness
/// says it is exiting — over the protocol, before it signals the leader — and
/// the shutdown that follows is then the owner's word: its fleet teardown
/// gives every swarm's environment up. An unannounced shutdown (a crashed or
/// killed client, a logout, an operator's signal) is not, and keeps them.
///
/// The announcement is held by the client that made it and withdrawn when
/// that client's connection closes: an owner that dies in its exit window
/// (or a client that detaches after an old-style announcement) leaves
/// nothing raised for a later, unannounced shutdown to mistake for its
/// word. A shutdown that already read it is unaffected — it decides once,
/// before its fleet step. The client is an opaque connection number.
pub trait OwnerExitAnnouncement: Send + Sync {
    fn announce(&self, client: u64);
    fn withdraw(&self, client: u64);
    fn announced(&self) -> bool;
}

/// Ends the environments this harness process created, emptied and kept
/// `retained` — a coordinator that crashed earlier left its box behind —
/// once the owner exits (#2070). Boxes an earlier process created are
/// `restored`, not this process's, and stay explicit-kill-only. The kills
/// run concurrently and each kill script is bounded by the command adapter,
/// so the owner's exit waits for one bound at most. Each answer names the
/// environment and how its end went.
pub trait RetainedEnvironmentTeardown: Send + Sync {
    fn end_emptied_retained(&self) -> PortFuture<'_, Vec<(String, Result<(), String>)>>;
}

/// Persists the session as part of shutdown, for the recorded reason.
pub trait ShutdownSessionPersistence: Send + Sync {
    fn persist_for_shutdown(&self, reason: ShutdownReason) -> PortFuture<'_, Result<(), String>>;
}

/// Milliseconds on a monotonic scale chosen by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShutdownInstant(pub u64);

pub trait ShutdownClock: Send + Sync {
    fn now(&self) -> ShutdownInstant;
}

/// Why composition is being told the harness may exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReadiness {
    /// The common teardown ran to completion for this reason.
    Completed(ShutdownReason),
    /// The teardown could not be driven to completion after its ACK was on
    /// the wire; composition must still exit (falling back to process exit)
    /// so the parent never sees an ACK followed by nothing.
    Abandoned {
        reason: ShutdownReason,
        detail: String,
    },
}

impl ExitReadiness {
    pub const fn reason(&self) -> ShutdownReason {
        match self {
            Self::Completed(reason) | Self::Abandoned { reason, .. } => *reason,
        }
    }
}

/// Tells composition the harness may now exit; composition owns the actual
/// process exit and the order in which runtimes stop.
pub trait CompositionExitReadiness: Send + Sync {
    fn signal_exit_ready(&self, readiness: ExitReadiness) -> PortFuture<'_, ()>;
}

/// The executed teardown, detached from whichever caller admitted it.
pub type ShutdownRun = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Runs the teardown independently of the caller's future, so a dropped
/// connection task can never abandon a shutdown whose ACK is already on the
/// wire. Composition supplies the runtime; the transaction owns the run.
pub trait ShutdownRunSpawner: Send + Sync {
    fn spawn_shutdown_run(&self, run: ShutdownRun);
}

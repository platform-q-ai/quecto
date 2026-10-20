//! The launching harness's registry of delegated agents: identity resolution,
//! the per-row teardown claims and observation of a row's compensation.

use super::PortFuture;
use crate::domain::agents::services::subagent_teardown::DelegatedAgentIdentity;
use std::fmt;

/// Why a delegated agent's row could not be resolved to one live identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// No row carries that uuid or live display label.
    Unknown,
    /// More than one live row answers to that display label.
    Ambiguous,
    /// The row is already exited or compensated.
    Exited,
    /// The row is not a delegated agent this harness can address: it was
    /// neither launched here nor reported with a launch generation.
    NotDelegated,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("not found in registry"),
            Self::Ambiguous => f.write_str("display label is ambiguous; use the uuid"),
            Self::Exited => f.write_str("already exited"),
            Self::NotDelegated => {
                f.write_str("not a delegated agent launched through this harness")
            }
        }
    }
}

/// Why a stopping claim was refused. Every refusal means no effect ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoppingClaimError {
    /// No row with that identity (uuid and generation) is known.
    Unknown,
    /// Another termination claimed the row and is still executing; a claim
    /// whose owner already returned is re-taken instead.
    AlreadyStopping,
    /// The row is already terminal.
    Exited,
}

impl fmt::Display for StoppingClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown delegated agent"),
            Self::AlreadyStopping => f.write_str("a termination is already in flight"),
            Self::Exited => f.write_str("already exited"),
        }
    }
}

/// What the harness observed about a child's end, in the words of the path
/// that observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitObservation {
    /// The owned process was reaped (exit status observed).
    ProcessExited,
    /// The monitor connection reached EOF or was reset.
    ConnectionClosed,
    /// The monitor connection was never accepted.
    NeverReachable,
}

/// Why a row's terminal effects run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationCause {
    /// The child ended on its own, observed this way.
    Exit(ExitObservation),
    /// An operator selected the agent for termination: its owner's word
    /// (#2206) — a plain container child's environment ends for good, a
    /// swarm its owner has not closed keeps its box.
    SelectedTermination,
    /// A one-shot parent is ending the run it was started for (#2206): it
    /// finished or hit its `--max-time`; never a failed run, a signal or a
    /// crash (those settle nothing). Its
    /// word for the children it launched — a plain container child's
    /// environment ends for good — but no close of any swarm, whose box is
    /// kept as for a harness shutdown.
    RunEnd,
    /// The whole fleet of direct children is being torn down by the harness's
    /// own shutdown (#1938), whatever triggered it: a termination signal, the
    /// last client leaving, a lost parent. Death by the parent's hand: no
    /// post-mortem — but possibly a crash, so a swarm its owner has not closed
    /// keeps its environment (#2070).
    FleetTeardown,
    /// The whole fleet is being torn down by an explicit act of its owner
    /// (#2070): delete-all, or a session transition whose target is already
    /// proven. The owner's swarms end with it and their environments are
    /// removed.
    OwnerTeardown,
    /// The environment the child is a member of is being killed by an
    /// explicit `kill_container` (#1939). Death by the parent's hand: no
    /// post-mortem; the environment's own kill belongs to the caller's
    /// already-held claim.
    EnvironmentKill,
    /// A launch failed after registration; `owns_environment` says whether
    /// the launch created the environment it joined.
    LaunchRollback { owns_environment: bool },
}

/// Result of claiming a row's terminal effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalClaim {
    /// This caller owns the compensation and must run it.
    Claimed,
    /// Another path already claimed (or finished) it; join instead.
    AlreadyClaimed,
}

/// Outcome of waiting for a row's compensation to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationObservation {
    /// The row's terminal effects have run.
    Compensated,
    /// The adapter's bound elapsed with the row still not compensated.
    TimedOut,
    /// No row with that identity exists (it was never registered, or was
    /// uncommitted).
    Unknown,
}

/// The launching harness's registry of delegated agents: identity
/// resolution, the per-row teardown claims that make every termination path
/// converge exactly once, and observation of a row's compensation.
pub trait DelegatedAgentRegistry: Send + Sync {
    /// Resolve an operator reference (uuid or live display label) to the
    /// one live delegated agent it names.
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError>;
    /// Mark the row stopping before any effect runs. Refused, with no
    /// effect, when the row is unknown, already stopping or terminal.
    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError>;
    /// Lift a stopping claim before anything reached the child, so a later
    /// trigger may try again. Idempotent.
    fn release_stopping(&self, target: &DelegatedAgentIdentity);
    /// Keep a stopping claim whose owner returned after effects reached the
    /// child without observing its end: the eventual exit is compensated
    /// as that termination (no post-mortem), and the claim is recorded as
    /// returned so a later trigger is not refused forever but re-takes it
    /// through `claim_stopping` and re-attempts the protocol. While the
    /// owner is executing, `claim_stopping` refuses `AlreadyStopping` and
    /// the later trigger joins. Idempotent.
    fn retain_stopping(&self, target: &DelegatedAgentIdentity);
    /// Claim the row's terminal effects. Exactly one caller ever gets
    /// `Claimed` for a row.
    fn claim_terminal(&self, target: &DelegatedAgentIdentity) -> TerminalClaim;
    /// Whether this harness still retains the child's unreaped process, in
    /// which case its reaper — not a connection edge — observes the exit.
    fn holds_process(&self, target: &DelegatedAgentIdentity) -> bool;
    /// Whether the row's terminal effects were already claimed (or ran):
    /// another path observed its exit. Never blocks, never claims.
    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool;
    /// Wait, within the adapter's bound, for the row's compensation.
    fn await_compensated<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation>;
}

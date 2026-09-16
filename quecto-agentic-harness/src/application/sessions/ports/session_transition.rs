//! The ports a session transition reaches beyond the conversation (D7
//! #1976, `Start fresh conversation` #1862): the fresh identity's impure
//! inputs, the departing session's delegated children (the fleet that
//! settles them and the roster that records them), and the loop runtime
//! the new key and the session-scoped settings are propagated to. The
//! transaction owns the order and every decision; the adapters perform
//! effects and decide nothing.
use std::future::Future;
use std::pin::Pin;

use super::session_runtime::TurnAccountingReset;
use crate::application::sessions::dto::FleetSettlementOutcome;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::workflow::WorkflowRunPersisted;

/// Port: a fresh user-chat identity in the current key format. The
/// adapter owns the impure inputs — the wall clock and a uniqueness token
/// (process id and per-process counter) — so two launches in the same
/// second, or two fresh sessions of one process, never share a key. The
/// transaction never claims the identity it is handed.
pub trait FreshSessionIdentityGenerator: Send + Sync {
    fn fresh_identity(&self) -> SessionIdentity;
}

/// Port: the fleet teardown that settles every direct child of the
/// departing session (#1938): each is asked to shut down over its edge,
/// concluded through the owned handle and compensated, under a bound. A
/// child whose end could not be observed is reported unsettled with its
/// claim lifted, never dropped.
pub trait FleetSettlement: Send + Sync {
    fn settle_fleet(&self) -> Pin<Box<dyn Future<Output = FleetSettlementOutcome> + Send + '_>>;
}

/// Port: the sub-agent roster of the loop as the transition sees it: how
/// many rows are live delegated agents (a child this harness addresses,
/// still live and not exited), and the wholesale replacement of the
/// roster once the transaction decided it holds records only.
pub trait DelegatedChildrenRoster: Send + Sync {
    fn live_delegated_rows(&self) -> usize;
    /// Drop every row; how many rows left the roster.
    fn clear_roster(&self) -> usize;
}

/// Port: what still holds the raw session key beside the active session —
/// the loop's tracker, the agent and its session-aware tools — adopts the
/// new identity.
pub trait SessionKeyPropagation: Send {
    fn session_key_changed(&mut self, identity: &SessionIdentity);
}

/// Port: the loop runtime a session switch moves besides the conversation
/// — its turn accounting, the key propagation, and the session-scoped
/// settings that must not follow the client into the new session: the
/// effort override (#1067) and the workflow run, reset for a fresh session
/// and restored from the saved one for a resume (D8 #1977). Whether a
/// reset or restore is made visible to clients (the tracker's generation
/// bumps only when the effort or the workflow snapshot actually changed) is
/// the adapter's rule, master-verbatim and contract-tested; the transaction
/// only orders it.
pub trait SessionSwitchRuntime: TurnAccountingReset + SessionKeyPropagation {
    fn reset_effort_to_default(&mut self);
    fn reset_workflow(&mut self);
    /// Replace the bound workflow engine's run with the one the resumed
    /// session recorded; a no-op without an engine.
    fn restore_workflow(&mut self, run: WorkflowRunPersisted);
}

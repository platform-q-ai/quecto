//! Boundary values of Start fresh conversation (#1862, D7 #1976) and of
//! the departing-children settlement every session transition runs
//! first: what the fleet reported, why a transition was refused, what a
//! fresh session is, and how starting one failed.
use super::SaveSessionError;
use crate::application::sessions::conversation_ledger::LedgerAdvance;
use crate::domain::ids::AgentUuid;
use crate::domain::session_identity::SessionIdentity;

/// Which transition the departing children are settled for; the label
/// the settlement logs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
    /// `new_session`: a fresh, unclaimed identity replaces the current one.
    Fresh,
    /// `resume_session`: a saved session replaces the current one.
    Resume,
}

impl SessionTransition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "new_session",
            Self::Resume => "resume_session",
        }
    }
}

/// What one fleet teardown run reported (#1938).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetSettlementOutcome {
    /// Every direct child settled; the counts are for the log.
    Settled(FleetSettled),
    /// Children whose end could not be observed within the bound; their
    /// rows stay live with the stopping claim lifted.
    Unsettled(Vec<(AgentUuid, String)>),
    /// The detached run was dropped before it completed; nothing was
    /// replaced.
    Interrupted,
}

/// The counts of a settled fleet run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FleetSettled {
    pub settled: usize,
    pub pruned: usize,
    /// This caller joined a run another trigger had started.
    pub joined: bool,
    /// Distinct rows the run moved out of the roster.
    pub removed: usize,
}

/// Why a session transition was refused before anything was replaced:
/// the current session, its key and its roster are kept as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionTransitionRefused {
    /// The fleet teardown left children unsettled: their rows keep their
    /// claims lifted and the current session stays as it is.
    Unsettled(Vec<(AgentUuid, String)>),
    /// The teardown run was interrupted; nothing was replaced.
    Interrupted,
    /// Live delegated rows exist but this harness has no fleet teardown to
    /// settle them with (a loop built without a teardown graph).
    NoFleetTeardown(usize),
    /// Live delegated rows remain after the fleet settled (a registration
    /// the run did not see): the roster is not replaced under them.
    LiveRowsRemain(usize),
}

impl std::fmt::Display for SessionTransitionRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsettled(children) => {
                let names: Vec<String> = children
                    .iter()
                    .map(|(uuid, detail)| format!("{uuid}: {detail}"))
                    .collect();
                write!(
                    f,
                    "{} subagent(s) could not be settled; the current session was kept: {}",
                    children.len(),
                    names.join("; ")
                )
            }
            Self::Interrupted => {
                f.write_str("subagent teardown was interrupted; the current session was kept")
            }
            Self::NoFleetTeardown(live) => write!(
                f,
                "{live} live subagent(s) but no fleet teardown is available; the current session was kept"
            ),
            Self::LiveRowsRemain(live) => write!(
                f,
                "{live} live subagent(s) remain after the teardown; the current session was kept"
            ),
        }
    }
}

impl std::error::Error for SessionTransitionRefused {}

/// A fresh session was started: the departing one was saved, the roster
/// replaced, the conversation, accounting and retention namespace reset,
/// and `identity` — unclaimed — installed everywhere the key is held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreshConversationStarted {
    pub identity: SessionIdentity,
    /// The ledger position after the reset, for the transports to announce.
    pub ledger: LedgerAdvance,
}

/// Why no fresh session was started. Every failure precedes the key
/// replacement: the current session, its key, its conversation and its
/// roster records are kept; children the fleet already settled stay
/// settled.
#[derive(Debug)]
pub enum StartFreshConversationError {
    Refused(SessionTransitionRefused),
    /// The departing session could not be saved.
    Save(SaveSessionError),
}

impl std::fmt::Display for StartFreshConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refused) => write!(f, "{refused}"),
            Self::Save(error) => write!(f, "failed to save current session: {error}"),
        }
    }
}

impl std::error::Error for StartFreshConversationError {}

#[cfg(test)]
#[path = "start_fresh_conversation_tests.rs"]
mod tests;

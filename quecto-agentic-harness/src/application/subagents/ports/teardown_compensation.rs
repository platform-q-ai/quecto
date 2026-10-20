//! A claimed row's terminal effects, run exactly once.

use super::PortFuture;
use super::delegated_agent_registry::TerminationCause;
use crate::domain::agents::services::subagent_teardown::DelegatedAgentIdentity;
use crate::domain::identity::ids::AgentUuid;

/// The rows a compensation removed from live membership: the target and
/// every descendant reported beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compensated {
    pub removed: Vec<AgentUuid>,
}

/// Runs a claimed row's terminal effects exactly once: environment cleanup
/// and membership removal, monitor and bridge teardown, removal of the row
/// and its reported subtree from live membership, one survivor broadcast
/// and one notification. Never signals a process.
pub trait TeardownCompensation: Send + Sync {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated>;

    /// Discard the record of every row whose terminal effects have already
    /// run (an exited tombstone), returning the discarded identities. Never
    /// touches a live or in-flight row and never signals anything (#1938).
    fn prune_terminal_rows(&self) -> PortFuture<'_, Vec<AgentUuid>>;
}

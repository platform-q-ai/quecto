//! [`OwnedChildTermination`] over the supervisor (#1936): the one fallback
//! for a directly owned child, after the caller's protocol attempt.
//!
//! The handle is resolved through this harness's own registry (uuid *and*
//! launch generation of a child it launched, whose slot the supervisor
//! still retains). Rows without a retained handle — script and container
//! members, merged descendants, restored rows, fixtures — conclude as
//! [`TerminationConclusion::NoRetainedHandle`] and nothing is signalled.
//! The protocol outcome the caller already observed is handed to the
//! supervisor as the attempt, so its TERM → wait → KILL ordering, at-most-
//! once dispatch and reap-safe classification (#1935) apply unchanged.
use std::sync::Arc;
use std::time::Duration;

use crate::application::subagents::ports::{
    ConclusionBudget, OwnedChildTermination, PortFuture, ProtocolAttempt, TerminationConclusion,
};
use crate::domain::subagent_teardown::DelegatedAgentIdentity;
use crate::infrastructure::tools::subagent_registry::SubagentRegistry;

use super::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor, ProtocolOutcome, TerminationBudget, TerminationOutcome,
};

/// A rolled-back child was never handed work: a short exit budget suffices
/// before the supervisor's fallback.
pub const ROLLBACK_BUDGET: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_secs(2),
    term_grace: Duration::from_secs(2),
    kill_grace: Duration::from_secs(2),
};

pub struct SupervisedChildTermination {
    registry: SubagentRegistry,
    standard: TerminationBudget,
    rollback: TerminationBudget,
}

impl SupervisedChildTermination {
    pub fn new(registry: SubagentRegistry) -> Self {
        Self {
            registry,
            standard: TerminationBudget::DEFAULT,
            rollback: ROLLBACK_BUDGET,
        }
    }

    /// Override both budgets (tests shorten the waits).
    pub fn with_budgets(
        mut self,
        standard: TerminationBudget,
        rollback: TerminationBudget,
    ) -> Self {
        self.standard = standard;
        self.rollback = rollback;
        self
    }

    /// The retained handle for `child`, only when the row is one this
    /// harness launched at exactly that generation.
    fn retained_handle(
        &self,
        child: &DelegatedAgentIdentity,
    ) -> Option<(ChildHandleId, Arc<OwnedChildSupervisor>)> {
        let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries.get(child.uuid.as_str()).or_else(|| {
            entries
                .values()
                .find(|entry| entry.agent_uuid == child.uuid)
        })?;
        if entry.launch_generation != Some(child.generation) {
            return None;
        }
        let handle = entry.owned_child?;
        let supervisor = entry.owned_child_supervisor.clone()?;
        supervisor.knows(handle).then_some((handle, supervisor))
    }
}

/// Conclude `handle` through `supervisor` after an already-observed protocol
/// attempt. Shared with the pre-registration launch rollback, which holds
/// its handle in the launch transaction rather than a registry row.
pub async fn conclude_retained(
    supervisor: &OwnedChildSupervisor,
    handle: ChildHandleId,
    attempt: ProtocolAttempt,
    budget: TerminationBudget,
) -> TerminationConclusion {
    let acknowledged = matches!(attempt, ProtocolAttempt::Acknowledged);
    let protocol = async move {
        match attempt {
            ProtocolAttempt::Acknowledged => ProtocolOutcome::Acknowledged,
            ProtocolAttempt::Negative(detail) => ProtocolOutcome::Negative(detail),
        }
    };
    match supervisor.terminate(handle, protocol, budget).await {
        // The child was alive to acknowledge and had already exited by the
        // time the supervisor looked: that is the protocol's exit, not a
        // child that was gone before the termination reached it.
        TerminationOutcome::AlreadyExited(_) if acknowledged => {
            TerminationConclusion::ExitedAfterProtocol
        }
        outcome => conclusion_of(outcome),
    }
}

pub fn conclusion_of(outcome: TerminationOutcome) -> TerminationConclusion {
    match outcome {
        TerminationOutcome::NoRetainedHandle => TerminationConclusion::NoRetainedHandle,
        TerminationOutcome::AlreadyExited(_) => TerminationConclusion::AlreadyExited,
        TerminationOutcome::ExitedAfterProtocol(_) => TerminationConclusion::ExitedAfterProtocol,
        TerminationOutcome::ExitedAfterTerm { .. } | TerminationOutcome::ExitedAfterKill { .. } => {
            TerminationConclusion::ExitedAfterFallback
        }
        TerminationOutcome::StillRunning { negative } => TerminationConclusion::StillRunning(
            format!("fallback signals sent after '{negative}' but no exit was observed"),
        ),
    }
}

impl OwnedChildTermination for SupervisedChildTermination {
    fn conclude<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
        budget: ConclusionBudget,
    ) -> PortFuture<'a, TerminationConclusion> {
        Box::pin(async move {
            let Some((handle, supervisor)) = self.retained_handle(child) else {
                return TerminationConclusion::NoRetainedHandle;
            };
            let budget = match budget {
                ConclusionBudget::Standard => self.standard,
                ConclusionBudget::Rollback => self.rollback,
            };
            conclude_retained(&supervisor, handle, attempt, budget).await
        })
    }
}

#[cfg(test)]
#[path = "owned_child_termination_tests.rs"]
mod tests;

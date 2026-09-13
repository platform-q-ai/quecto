//! Rollback of a launch that failed after its child was registered (#1936):
//! the initial prompt could not be delivered, or the environment refused
//! the member.
//!
//! The registered child is a direct child like any other, so it is ended
//! the same way: claim the row stopping, ask it to shut down over its own
//! edge, let the owned-handle fallback conclude a locally owned process
//! (protocol first, a signal only after a negative outcome or an exit
//! timeout, nothing at all without a retained handle), then claim and run
//! the launch-rollback compensation — or join the path that already ran it.
//! A rollback never waits on a child this harness does not own: an
//! acknowledged script-managed member is compensated at once.
use std::sync::Arc;

use crate::domain::subagent_teardown::ShutdownReason;

use super::super::dto::{CompensateFailedLaunchRequest, FailedLaunchCompensated};
use super::super::ports::{
    ConclusionBudget, DelegatedAgentRegistry, DirectChildRouting, OwnedChildTermination,
    ProtocolAttempt, StoppingClaimError, TeardownCompensation, TerminalClaim, TerminationCause,
    TerminationConclusion,
};

pub struct CompensateFailedLaunchPorts {
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub routing: Arc<dyn DirectChildRouting>,
    pub termination: Arc<dyn OwnedChildTermination>,
    pub compensation: Arc<dyn TeardownCompensation>,
}

pub struct CompensateFailedLaunch {
    ports: CompensateFailedLaunchPorts,
}

impl CompensateFailedLaunch {
    pub fn new(ports: CompensateFailedLaunchPorts) -> Self {
        Self { ports }
    }

    /// Never fails: a rollback must complete. A row already terminal (or
    /// already being ended by another path) is joined, not ended twice.
    pub async fn execute(&self, request: CompensateFailedLaunchRequest) -> FailedLaunchCompensated {
        let CompensateFailedLaunchRequest {
            child,
            owns_environment,
        } = request;
        let cause = TerminationCause::LaunchRollback { owns_environment };
        match self.ports.registry.claim_stopping(&child, cause) {
            Ok(()) => {}
            // Another termination owns the row (in flight, or already
            // terminal): this rollback only joins its compensation. It never
            // claims the terminal effects itself — the owner observes the
            // exit, and a row whose process may still be alive must not be
            // compensated from here.
            Err(StoppingClaimError::AlreadyStopping | StoppingClaimError::Exited) => {
                let _ = self.ports.registry.await_compensated(&child).await;
                return FailedLaunchCompensated {
                    conclusion: TerminationConclusion::NoRetainedHandle,
                    removed: Vec::new(),
                };
            }
            // Never registered (or already uncommitted): nothing to end.
            Err(StoppingClaimError::Unknown) => {
                return FailedLaunchCompensated {
                    conclusion: TerminationConclusion::NoRetainedHandle,
                    removed: Vec::new(),
                };
            }
        }
        let attempt = match self
            .ports
            .routing
            .shutdown_child(&child, ShutdownReason::OperatorRequest)
            .await
        {
            Ok(()) => ProtocolAttempt::Acknowledged,
            Err(error) => ProtocolAttempt::Negative(error.to_string()),
        };
        let conclusion = self
            .ports
            .termination
            .conclude(&child, attempt, ConclusionBudget::Rollback)
            .await;
        // Only this rollback's own claim reaches the terminal effects; a
        // reaper that observed the exit first is joined.
        let removed = match self.ports.registry.claim_terminal(&child) {
            TerminalClaim::Claimed => {
                self.ports
                    .compensation
                    .compensate(&child, cause)
                    .await
                    .removed
            }
            TerminalClaim::AlreadyClaimed => {
                let _ = self.ports.registry.await_compensated(&child).await;
                Vec::new()
            }
        };
        FailedLaunchCompensated {
            conclusion,
            removed,
        }
    }
}

#[cfg(test)]
#[path = "compensate_failed_launch_tests.rs"]
mod tests;

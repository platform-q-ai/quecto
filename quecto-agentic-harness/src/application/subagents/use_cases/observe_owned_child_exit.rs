//! A direct child ended on its own (#1936): the reaper reaped its process,
//! or the monitor connection closed / never opened.
//!
//! One rule makes every path converge: the row's terminal effects are
//! claimed exactly once. The path that wins the claim runs the compensation;
//! every other observation of the same end joins it. A connection-level
//! observation of a child whose process this harness still retains defers to
//! the reaper — the process is alive until it is reaped, and removing its
//! row first would leave a live, untracked child — so no path ever removes a
//! row before the exit it stands for was observed.
use std::sync::Arc;

use super::super::dto::{ObserveOwnedChildExitRequest, ObservedExit};
use super::super::ports::{
    DelegatedAgentRegistry, ExitObservation, TeardownCompensation, TerminalClaim, TerminationCause,
};

pub struct ObserveOwnedChildExit {
    registry: Arc<dyn DelegatedAgentRegistry>,
    compensation: Arc<dyn TeardownCompensation>,
}

impl ObserveOwnedChildExit {
    pub fn new(
        registry: Arc<dyn DelegatedAgentRegistry>,
        compensation: Arc<dyn TeardownCompensation>,
    ) -> Self {
        Self {
            registry,
            compensation,
        }
    }

    pub async fn execute(&self, request: ObserveOwnedChildExitRequest) -> ObservedExit {
        let ObserveOwnedChildExitRequest { child, observation } = request;
        let connection_level = matches!(
            observation,
            ExitObservation::ConnectionClosed | ExitObservation::NeverReachable
        );
        if connection_level && self.registry.holds_process(&child) {
            return ObservedExit::DeferredToProcessExit;
        }
        match self.registry.claim_terminal(&child) {
            TerminalClaim::Claimed => {
                let compensated = self
                    .compensation
                    .compensate(&child, TerminationCause::Exit(observation))
                    .await;
                ObservedExit::Compensated {
                    removed: compensated.removed,
                }
            }
            TerminalClaim::AlreadyClaimed => {
                ObservedExit::Joined(self.registry.await_compensated(&child).await)
            }
        }
    }
}

#[cfg(test)]
#[path = "observe_owned_child_exit_tests.rs"]
mod tests;

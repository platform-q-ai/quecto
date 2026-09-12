//! Selected-descendant termination, resolved one direct edge at a time (#1934).
//!
//! The receiving harness never shuts itself down here. It either asks the
//! direct child that *is* the target to shut down, or forwards the command
//! to the direct child whose subtree contains the target with one hop
//! consumed. Every rejection happens before any port is touched.
use std::sync::Arc;

use crate::domain::subagent_teardown::{
    ShutdownReason, TerminationRoute, resolve_termination_route,
};

use super::super::dto::{
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationRouted,
};
use super::super::ports::{
    DelegatedAgentRegistry, DirectChildRouting, StoppingClaimError, SubagentLifecycleRepository,
    TerminationCause,
};

pub struct TerminateDelegatedAgent {
    lifecycle: Arc<dyn SubagentLifecycleRepository>,
    routing: Arc<dyn DirectChildRouting>,
    /// When present, the target's row is claimed stopping with the
    /// selected-termination intent before its edge is routed (#1936), so
    /// the receiver's own reaper or monitor honours that intent when it
    /// observes the exit: no post-mortem inspect, no "exited unexpectedly"
    /// note for a child an ancestor selected.
    registry: Option<Arc<dyn DelegatedAgentRegistry>>,
}

impl TerminateDelegatedAgent {
    pub fn new(
        lifecycle: Arc<dyn SubagentLifecycleRepository>,
        routing: Arc<dyn DirectChildRouting>,
    ) -> Self {
        Self {
            lifecycle,
            routing,
            registry: None,
        }
    }

    pub fn with_registry(mut self, registry: Arc<dyn DelegatedAgentRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Claim the target stopping for this edge. `true` when this call took
    /// the claim (and must lift it if the edge fails); a claim another
    /// path already holds is left to that path.
    fn claim_target(
        &self,
        target: &crate::domain::subagent_teardown::DelegatedAgentIdentity,
    ) -> bool {
        let Some(registry) = &self.registry else {
            return false;
        };
        match registry.claim_stopping(target, TerminationCause::SelectedTermination) {
            Ok(()) => true,
            Err(
                StoppingClaimError::AlreadyStopping
                | StoppingClaimError::Exited
                | StoppingClaimError::Unknown,
            ) => false,
        }
    }

    fn release_target(
        &self,
        target: &crate::domain::subagent_teardown::DelegatedAgentIdentity,
        claimed: bool,
    ) {
        if claimed {
            if let Some(registry) = &self.registry {
                registry.release_stopping(target);
            }
        }
    }

    pub async fn execute(
        &self,
        request: TerminateDelegatedAgentRequest,
    ) -> Result<TerminationRouted, TerminateDelegatedAgentError> {
        // A frozen or terminated harness is on its way out; its subtree is
        // already being torn down and must not be re-routed underneath.
        if !self.lifecycle.lifecycle().accepts_new_work() {
            return Err(TerminateDelegatedAgentError::NotAccepting);
        }
        let lineage = self.lifecycle.lineage();
        let route = resolve_termination_route(&lineage, &request.target, request.remaining_depth)
            .map_err(TerminateDelegatedAgentError::Rejected)?;
        // The route is affirmed: claim the target before the edge, and lift
        // the claim again if the edge cannot be delivered.
        let claimed = self.claim_target(&request.target);
        let routed = match route {
            TerminationRoute::ShutdownDirectChild(child) => {
                debug_assert_eq!(child.uuid, request.target.uuid);
                self.routing
                    .shutdown_child(&child, ShutdownReason::SelectedTermination)
                    .await
                    .map_err(|error| TerminateDelegatedAgentError::ChildUnreachable {
                        child: child.uuid.clone(),
                        detail: error.to_string(),
                    })
                    .map(|()| TerminationRouted::ShutdownRequested { child })
            }
            TerminationRoute::ForwardToDirectChild {
                via,
                remaining_depth,
            } => {
                debug_assert!(remaining_depth < request.remaining_depth);
                debug_assert_ne!(via.uuid, request.target.uuid);
                self.routing
                    .forward_termination(&via, &request.target, remaining_depth)
                    .await
                    .map_err(|error| TerminateDelegatedAgentError::ChildUnreachable {
                        child: via.uuid.clone(),
                        detail: error.to_string(),
                    })
                    .map(|()| TerminationRouted::Forwarded {
                        via,
                        remaining_depth,
                    })
            }
        };
        if routed.is_err() {
            self.release_target(&request.target, claimed);
        }
        routed
    }
}

#[cfg(test)]
#[path = "terminate_delegated_agent_tests.rs"]
mod tests;

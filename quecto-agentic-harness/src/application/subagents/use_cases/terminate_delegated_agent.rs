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
use super::super::ports::{DirectChildRouting, SubagentLifecycleRepository};

pub struct TerminateDelegatedAgent {
    lifecycle: Arc<dyn SubagentLifecycleRepository>,
    routing: Arc<dyn DirectChildRouting>,
}

impl TerminateDelegatedAgent {
    pub fn new(
        lifecycle: Arc<dyn SubagentLifecycleRepository>,
        routing: Arc<dyn DirectChildRouting>,
    ) -> Self {
        Self { lifecycle, routing }
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
        match route {
            TerminationRoute::ShutdownDirectChild(child) => {
                debug_assert_eq!(child.uuid, request.target.uuid);
                self.routing
                    .shutdown_child(&child, ShutdownReason::SelectedTermination)
                    .await
                    .map_err(|error| TerminateDelegatedAgentError::ChildUnreachable {
                        child: child.uuid.clone(),
                        detail: error.to_string(),
                    })?;
                Ok(TerminationRouted::ShutdownRequested { child })
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
                    })?;
                Ok(TerminationRouted::Forwarded {
                    via,
                    remaining_depth,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "terminate_delegated_agent_tests.rs"]
mod tests;

//! Operator-selected termination of one delegated agent (#1936, #1882).
//!
//! The order is the policy:
//!
//! 1. resolve the reference to one live identity (uuid *and* generation);
//! 2. claim the row as stopping — before any effect, so a concurrent kill,
//!    exit observation or rollback sees the claim and converges;
//! 3. route exactly one edge through [`TerminateDelegatedAgent`]: a direct
//!    child gets self shutdown, a deeper target is forwarded through its
//!    direct ancestor, which stays alive (an intermediate is never sent self
//!    shutdown for a deeper target);
//! 4. observe the end: for a directly owned child the owned-handle fallback
//!    concludes it (protocol first; a signal only after a negative outcome
//!    or an exit timeout); for anything this harness does not own the only
//!    observation is the row's compensation, reported truthfully when it
//!    does not arrive;
//! 5. only then claim and run the terminal compensation (cleanup,
//!    membership, subtree removal, one broadcast) — or join the path that
//!    already ran it.
//!
//! Every refusal or failure lifts the stopping claim: the registry is left
//! as it was, and a later trigger may try again.
use std::sync::Arc;

use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LineageSnapshot, RoutingDepth, TerminationRouteError,
};

use super::super::dto::{
    KillDelegatedAgentError, KillDelegatedAgentOutcome, KillDelegatedAgentRequest,
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationResult,
    TerminationRouted,
};
use super::super::ports::{
    CompensationObservation, ConclusionBudget, DelegatedAgentRegistry, OwnedChildTermination,
    ProtocolAttempt, ResolutionError, StoppingClaimError, SubagentLifecycleRepository,
    TeardownCompensation, TerminalClaim, TerminationCause, TerminationConclusion,
};
use super::terminate_delegated_agent::TerminateDelegatedAgent;

pub struct KillDelegatedAgentPorts {
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub lifecycle: Arc<dyn SubagentLifecycleRepository>,
    pub termination: Arc<dyn OwnedChildTermination>,
    pub compensation: Arc<dyn TeardownCompensation>,
}

pub struct KillDelegatedAgent {
    route: Arc<TerminateDelegatedAgent>,
    ports: KillDelegatedAgentPorts,
}

/// What the routed edge established about the target's end before the
/// fallback or the compensation observation was consulted.
enum Edge {
    /// The target is a direct child and the protocol attempt concluded.
    Direct(ProtocolAttempt),
    /// The target lives deeper; its direct ancestor accepted the route.
    Forwarded,
}

impl KillDelegatedAgent {
    pub fn new(route: Arc<TerminateDelegatedAgent>, ports: KillDelegatedAgentPorts) -> Self {
        Self { route, ports }
    }

    pub async fn execute(
        &self,
        request: KillDelegatedAgentRequest,
    ) -> Result<KillDelegatedAgentOutcome, KillDelegatedAgentError> {
        let target = self
            .ports
            .registry
            .resolve(&request.reference)
            .map_err(KillDelegatedAgentError::Unresolved)?;
        self.ports
            .registry
            .claim_stopping(&target, TerminationCause::SelectedTermination)
            .map_err(|error| match error {
                StoppingClaimError::AlreadyStopping => KillDelegatedAgentError::AlreadyStopping,
                StoppingClaimError::Unknown => {
                    KillDelegatedAgentError::Unresolved(ResolutionError::Unknown)
                }
                StoppingClaimError::Exited => {
                    KillDelegatedAgentError::Unresolved(ResolutionError::Exited)
                }
            })?;
        // The lineage the route is resolved against, captured before the
        // edge so the reported subtree is the one the route saw.
        let lineage = self.ports.lifecycle.lineage();
        match self.terminate(&target, lineage).await {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.ports.registry.release_stopping(&target);
                Err(error)
            }
        }
    }

    async fn terminate(
        &self,
        target: &DelegatedAgentIdentity,
        lineage: LineageSnapshot,
    ) -> Result<KillDelegatedAgentOutcome, KillDelegatedAgentError> {
        let edge = self.route_one_edge(target).await?;
        let result = match edge {
            Edge::Direct(attempt) => self.conclude_direct(target, attempt).await?,
            Edge::Forwarded => {
                // Nothing this harness owns ends a nested target: the only
                // truthful observation is its row's compensation, which the
                // intermediate's reported snapshot drives.
                self.observe_compensated(target).await?;
                TerminationResult::Graceful
            }
        };
        let removed = self.compensate(target, &lineage).await?;
        Ok(KillDelegatedAgentOutcome {
            target: target.clone(),
            result,
            removed,
        })
    }

    async fn route_one_edge(
        &self,
        target: &DelegatedAgentIdentity,
    ) -> Result<Edge, KillDelegatedAgentError> {
        let request = TerminateDelegatedAgentRequest {
            target: target.clone(),
            remaining_depth: RoutingDepth::new(RoutingDepth::MAX_HOPS)
                .expect("the maximum hop count is a valid depth"),
        };
        match self.route.execute(request).await {
            Ok(TerminationRouted::ShutdownRequested { child }) => {
                debug_assert_eq!(&child, target, "self shutdown goes only to the target");
                Ok(Edge::Direct(ProtocolAttempt::Acknowledged))
            }
            Ok(TerminationRouted::Forwarded { via, .. }) => {
                debug_assert_ne!(via.uuid, target.uuid, "an intermediate is never the target");
                Ok(Edge::Forwarded)
            }
            Err(TerminateDelegatedAgentError::ChildUnreachable { child, detail }) => {
                if child == target.uuid {
                    Ok(Edge::Direct(ProtocolAttempt::Negative(detail)))
                } else {
                    Err(KillDelegatedAgentError::RouteUnreachable { via: child, detail })
                }
            }
            Err(TerminateDelegatedAgentError::Rejected(error)) => {
                Err(KillDelegatedAgentError::Rejected(error))
            }
            Err(TerminateDelegatedAgentError::NotAccepting) => {
                Err(KillDelegatedAgentError::NotAccepting)
            }
        }
    }

    /// Conclude a direct child: the owned-handle fallback if this harness
    /// retains its process, otherwise the compensation observation alone.
    async fn conclude_direct(
        &self,
        target: &DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
    ) -> Result<TerminationResult, KillDelegatedAgentError> {
        let acknowledged = matches!(attempt, ProtocolAttempt::Acknowledged);
        let negative = match &attempt {
            ProtocolAttempt::Acknowledged => None,
            ProtocolAttempt::Negative(detail) => Some(detail.clone()),
        };
        match self
            .ports
            .termination
            .conclude(target, attempt, ConclusionBudget::Standard)
            .await
        {
            TerminationConclusion::NoRetainedHandle => {
                if let Some(detail) = negative {
                    // The edge may have failed because the child was already
                    // gone: its reaper (or monitor) observed the exit, claimed
                    // the terminal effects with this kill's intent and
                    // retired the handle before the protocol answered. That
                    // is an exited child, not a failed termination.
                    if self.ports.registry.terminal_claimed(target) {
                        self.observe_compensated(target).await?;
                        return Ok(TerminationResult::AlreadyExited);
                    }
                    // Not owned and not reachable: nothing more is
                    // authorised, so the failure is reported as it is.
                    return Err(KillDelegatedAgentError::Failed { detail });
                }
                debug_assert!(acknowledged);
                self.observe_compensated(target).await?;
                Ok(TerminationResult::Graceful)
            }
            TerminationConclusion::AlreadyExited => Ok(TerminationResult::AlreadyExited),
            TerminationConclusion::ExitedAfterProtocol => Ok(TerminationResult::Graceful),
            TerminationConclusion::ExitedAfterFallback => Ok(TerminationResult::Fallback),
            TerminationConclusion::StillRunning(detail) => {
                Err(KillDelegatedAgentError::Failed { detail })
            }
        }
    }

    async fn observe_compensated(
        &self,
        target: &DelegatedAgentIdentity,
    ) -> Result<(), KillDelegatedAgentError> {
        match self.ports.registry.await_compensated(target).await {
            CompensationObservation::Compensated => Ok(()),
            CompensationObservation::TimedOut => Err(KillDelegatedAgentError::Failed {
                detail: "acknowledged but its exit was not observed within the bound".into(),
            }),
            CompensationObservation::Unknown => Err(KillDelegatedAgentError::Rejected(
                TerminationRouteError::UnknownTarget(target.uuid.clone()),
            )),
        }
    }

    /// The exit was observed: claim the terminal effects, or join the path
    /// (reaper, monitor, reported snapshot) that already claimed them.
    async fn compensate(
        &self,
        target: &DelegatedAgentIdentity,
        lineage: &LineageSnapshot,
    ) -> Result<Vec<crate::domain::ids::AgentUuid>, KillDelegatedAgentError> {
        match self.ports.registry.claim_terminal(target) {
            TerminalClaim::Claimed => Ok(self
                .ports
                .compensation
                .compensate(target, TerminationCause::SelectedTermination)
                .await
                .removed),
            TerminalClaim::AlreadyClaimed => {
                self.observe_compensated(target).await?;
                Ok(subtree(lineage, target))
            }
        }
    }
}

/// The target and every descendant beneath it in `lineage`, target first.
fn subtree(
    lineage: &LineageSnapshot,
    target: &DelegatedAgentIdentity,
) -> Vec<crate::domain::ids::AgentUuid> {
    let mut removed = vec![target.uuid.clone()];
    let mut index = 0;
    while index < removed.len() {
        let parent = removed[index].clone();
        for record in &lineage.records {
            if record.parent == parent && !removed.contains(&record.identity.uuid) {
                removed.push(record.identity.uuid.clone());
            }
        }
        index += 1;
    }
    removed
}

#[cfg(test)]
#[path = "kill_delegated_agent_tests.rs"]
mod tests;

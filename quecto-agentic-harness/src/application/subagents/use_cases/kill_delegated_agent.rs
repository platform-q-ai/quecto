//! Operator-selected termination of one delegated agent (#1936, #1882).
//!
//! The order is the policy:
//!
//! 1. resolve the reference to one live identity (uuid *and* generation);
//! 2. claim the row as stopping — before any effect, so a concurrent kill,
//!    exit observation or rollback sees the claim and converges;
//! 3. route exactly one edge through [`TerminateDelegatedAgent`], with the
//!    depth the lineage says the route needs: a direct child is concluded
//!    there as this harness's own (protocol, observed exit, owned-handle
//!    fallback only when the protocol did not suffice); a deeper target is
//!    forwarded through its direct ancestor, which stays alive, and the
//!    owner's result comes back with the hop's answer;
//! 4. only then claim and run the terminal compensation (cleanup,
//!    membership, subtree removal, one broadcast) — or join the path that
//!    already ran it.
//!
//! A refusal before anything reached the agent lifts the stopping claim
//! and leaves the registry as it was. A failure after effects were
//! dispatched keeps the claim, so the eventual exit is compensated as this
//! kill rather than post-mortemed as a natural one.
use std::sync::Arc;

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LineageSnapshot, RoutingDepth, TerminationRouteError, route_length,
};

use super::super::dto::{
    KillDelegatedAgentError, KillDelegatedAgentOutcome, KillDelegatedAgentRequest,
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationResult,
    TerminationRouted,
};
use super::super::ports::{
    CompensationObservation, DelegatedAgentRegistry, DownstreamRejection, ResolutionError,
    StoppingClaimError, SubagentLifecycleRepository, TeardownCompensation, TerminalClaim,
    TerminationCause,
};
use super::terminate_delegated_agent::TerminateDelegatedAgent;

pub struct KillDelegatedAgentPorts {
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub lifecycle: Arc<dyn SubagentLifecycleRepository>,
    pub compensation: Arc<dyn TeardownCompensation>,
}

pub struct KillDelegatedAgent {
    route: Arc<TerminateDelegatedAgent>,
    ports: KillDelegatedAgentPorts,
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
        match self.terminate(&target, &lineage).await {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                if !effects_dispatched(&error) {
                    self.ports.registry.release_stopping(&target);
                }
                Err(error)
            }
        }
    }

    async fn terminate(
        &self,
        target: &DelegatedAgentIdentity,
        lineage: &LineageSnapshot,
    ) -> Result<KillDelegatedAgentOutcome, KillDelegatedAgentError> {
        let result = self.route_one_edge(target, lineage).await?;
        let removed = self.compensate(target, lineage).await?;
        Ok(KillDelegatedAgentOutcome {
            target: target.clone(),
            result,
            removed,
        })
    }

    /// The depth the route needs: exactly the edges between this harness
    /// and the target, so every hop's bound is sized to the real route. A
    /// lineage the walk refuses gets the maximum and is refused again, with
    /// the same error, by the route itself.
    fn depth_for(lineage: &LineageSnapshot, target: &DelegatedAgentIdentity) -> RoutingDepth {
        route_length(lineage, target)
            .ok()
            .and_then(|hops| RoutingDepth::new(hops).ok())
            .unwrap_or_else(|| {
                RoutingDepth::new(RoutingDepth::MAX_HOPS)
                    .expect("the maximum hop count is a valid depth")
            })
    }

    async fn route_one_edge(
        &self,
        target: &DelegatedAgentIdentity,
        lineage: &LineageSnapshot,
    ) -> Result<TerminationResult, KillDelegatedAgentError> {
        let request = TerminateDelegatedAgentRequest {
            target: target.clone(),
            remaining_depth: Self::depth_for(lineage, target),
        };
        match self.route.execute(request).await {
            Ok(TerminationRouted::ShutdownRequested { child, result }) => {
                debug_assert_eq!(&child, target, "self shutdown goes only to the target");
                self.observed_or_relayed(target, result).await
            }
            Ok(TerminationRouted::Forwarded { via, result, .. }) => {
                debug_assert_ne!(via.uuid, target.uuid, "an intermediate is never the target");
                self.observed_or_relayed(target, result).await
            }
            // The end was observed before the route could act on it.
            Err(TerminateDelegatedAgentError::TargetAlreadyExited(_))
            | Err(TerminateDelegatedAgentError::Downstream {
                rejection: DownstreamRejection::AlreadyExited,
                ..
            }) => Ok(TerminationResult::AlreadyExited),
            Err(error) => Err(map_route_error(target, error)),
        }
    }

    /// A result the owner reported, or — for a bare route that observed
    /// nothing — the row's compensation as the only truthful observation.
    async fn observed_or_relayed(
        &self,
        target: &DelegatedAgentIdentity,
        result: Option<TerminationResult>,
    ) -> Result<TerminationResult, KillDelegatedAgentError> {
        match result {
            Some(result) => Ok(result),
            None => {
                self.observe_compensated(target).await?;
                Ok(TerminationResult::Graceful)
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
                effects_dispatched: true,
            }),
            CompensationObservation::Unknown => Err(KillDelegatedAgentError::Rejected(
                TerminationRouteError::UnknownTarget(target.uuid.clone()),
            )),
        }
    }

    /// The exit was observed: claim the terminal effects, or join the path
    /// (the owner's conclusion, the reaper, the monitor, a reported
    /// snapshot) that already claimed them.
    async fn compensate(
        &self,
        target: &DelegatedAgentIdentity,
        lineage: &LineageSnapshot,
    ) -> Result<Vec<AgentUuid>, KillDelegatedAgentError> {
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

/// Whether an error means something reached the agent, so its stopping
/// claim must be kept for the exit that will follow.
fn effects_dispatched(error: &KillDelegatedAgentError) -> bool {
    match error {
        KillDelegatedAgentError::Failed {
            effects_dispatched, ..
        } => *effects_dispatched,
        KillDelegatedAgentError::Unresolved(_)
        | KillDelegatedAgentError::AlreadyStopping
        | KillDelegatedAgentError::Rejected(_)
        | KillDelegatedAgentError::NotAccepting
        | KillDelegatedAgentError::RouteUnreachable { .. }
        | KillDelegatedAgentError::DownstreamRejected { .. } => false,
    }
}

fn map_route_error(
    target: &DelegatedAgentIdentity,
    error: TerminateDelegatedAgentError,
) -> KillDelegatedAgentError {
    match error {
        TerminateDelegatedAgentError::Rejected(error) => KillDelegatedAgentError::Rejected(error),
        TerminateDelegatedAgentError::NotAccepting => KillDelegatedAgentError::NotAccepting,
        TerminateDelegatedAgentError::ChildUnreachable { child, detail } => {
            if child == target.uuid {
                KillDelegatedAgentError::Failed {
                    detail,
                    effects_dispatched: false,
                }
            } else {
                KillDelegatedAgentError::RouteUnreachable { via: child, detail }
            }
        }
        TerminateDelegatedAgentError::TerminationFailed { detail, .. } => {
            KillDelegatedAgentError::Failed {
                detail,
                effects_dispatched: true,
            }
        }
        TerminateDelegatedAgentError::Downstream {
            rejection: DownstreamRejection::Failed(detail),
            ..
        } => KillDelegatedAgentError::Failed {
            detail,
            effects_dispatched: true,
        },
        TerminateDelegatedAgentError::Downstream { via, rejection } => {
            KillDelegatedAgentError::DownstreamRejected {
                via,
                detail: rejection.to_string(),
            }
        }
        // The caller turns this into a result before mapping errors; kept
        // total so a future route error cannot fall through silently.
        TerminateDelegatedAgentError::TargetAlreadyExited(_) => {
            KillDelegatedAgentError::Unresolved(ResolutionError::Exited)
        }
    }
}

/// The target and every descendant beneath it in `lineage`, target first.
fn subtree(lineage: &LineageSnapshot, target: &DelegatedAgentIdentity) -> Vec<AgentUuid> {
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

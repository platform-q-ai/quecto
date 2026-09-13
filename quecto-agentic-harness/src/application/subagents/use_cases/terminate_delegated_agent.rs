//! Selected-descendant termination, resolved one direct edge at a time
//! (#1934, #1936).
//!
//! The receiving harness never shuts itself down here. It either ends the
//! direct child that *is* the target — it is that child's owner, so it runs
//! the whole conclusion: protocol first, the exit observed within a bound,
//! the owned-handle fallback only when the protocol did not suffice, then
//! the row's exactly-once compensation — or forwards the command to the
//! direct child whose subtree contains the target with one hop consumed and
//! relays what that hop answered. Every rejection happens before any port
//! is touched.
use std::sync::Arc;

use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, ShutdownReason, TerminationRoute, TerminationRouteError,
    resolve_termination_route,
};

use super::super::dto::{
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationResult,
    TerminationRouted,
};
use super::super::ports::{
    ChildRoutingError, CompensationObservation, ConclusionBudget, DelegatedAgentRegistry,
    DirectChildRouting, OwnedChildTermination, ProtocolAttempt, StoppingClaimError,
    SubagentLifecycleRepository, TeardownCompensation, TerminalClaim, TerminationCause,
    TerminationConclusion,
};

/// What the owner of a direct child concludes its end with: the fallback
/// over the retained handle and the row's terminal compensation, both
/// observed through the registry.
pub struct OwnerConclusionPorts {
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub termination: Arc<dyn OwnedChildTermination>,
    pub compensation: Arc<dyn TeardownCompensation>,
}

pub struct TerminateDelegatedAgent {
    lifecycle: Arc<dyn SubagentLifecycleRepository>,
    routing: Arc<dyn DirectChildRouting>,
    /// When present, the target's row is claimed stopping with the
    /// selected-termination intent before its edge is routed, so the
    /// receiver's own reaper or monitor honours that intent when it
    /// observes the exit: no post-mortem inspect, no "exited unexpectedly"
    /// note for a child an ancestor selected.
    registry: Option<Arc<dyn DelegatedAgentRegistry>>,
    /// When present, a direct child that is the target is concluded here,
    /// as its owner. Absent only in bare routing rigs.
    owner: Option<OwnerConclusionPorts>,
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
            owner: None,
        }
    }

    pub fn with_registry(mut self, registry: Arc<dyn DelegatedAgentRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Conclude a direct child that is the target here (this harness owns
    /// it). The ports' registry is also the claim registry.
    pub fn with_owner_conclusion(mut self, owner: OwnerConclusionPorts) -> Self {
        self.registry = Some(owner.registry.clone());
        self.owner = Some(owner);
        self
    }

    /// Claim the target stopping for this edge. `true` when this call took
    /// the claim (and must lift it if nothing reaches the target); a claim
    /// another path already holds is left to that path.
    fn claim_target(&self, target: &DelegatedAgentIdentity) -> bool {
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

    fn release_target(&self, target: &DelegatedAgentIdentity, claimed: bool) {
        if claimed {
            if let Some(registry) = &self.registry {
                registry.release_stopping(target);
            }
        }
    }

    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool {
        self.registry
            .as_ref()
            .is_some_and(|registry| registry.terminal_claimed(target))
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
        let route =
            match resolve_termination_route(&lineage, &request.target, request.remaining_depth) {
                Ok(route) => route,
                // A target this harness no longer lists because it already
                // observed its end is exited, not unknown.
                Err(TerminationRouteError::UnknownTarget(uuid))
                    if self.terminal_claimed(&request.target) =>
                {
                    return Err(TerminateDelegatedAgentError::TargetAlreadyExited(uuid));
                }
                Err(error) => return Err(TerminateDelegatedAgentError::Rejected(error)),
            };
        // The route is affirmed: claim the target before the edge.
        let claimed = self.claim_target(&request.target);
        match route {
            TerminationRoute::ShutdownDirectChild(child) => {
                debug_assert_eq!(child.uuid, request.target.uuid);
                self.shut_down_owned_child(child, claimed).await
            }
            TerminationRoute::ForwardToDirectChild {
                via,
                remaining_depth,
            } => {
                debug_assert!(remaining_depth < request.remaining_depth);
                debug_assert_ne!(via.uuid, request.target.uuid);
                let answer = self
                    .routing
                    .forward_termination(&via, &request.target, remaining_depth)
                    .await;
                // An intermediate holds no authority over the target's end:
                // whatever the hop answered — a relayed result, a refusal,
                // a timeout — its own claim is lifted, and the row ends
                // through the owner's reported snapshot.
                self.release_target(&request.target, claimed);
                match answer {
                    Ok(result) => Ok(TerminationRouted::Forwarded {
                        via,
                        remaining_depth,
                        result,
                    }),
                    Err(ChildRoutingError::Downstream(rejection)) => {
                        Err(TerminateDelegatedAgentError::Downstream {
                            via: via.uuid,
                            rejection,
                        })
                    }
                    Err(
                        error @ (ChildRoutingError::NotADirectChild
                        | ChildRoutingError::Unreachable(_)),
                    ) => Err(TerminateDelegatedAgentError::ChildUnreachable {
                        child: via.uuid,
                        detail: error.to_string(),
                    }),
                }
            }
        }
    }

    /// The target is this harness's direct child: ask it over its edge,
    /// then — as its owner — observe its end before its row goes.
    async fn shut_down_owned_child(
        &self,
        child: DelegatedAgentIdentity,
        claimed: bool,
    ) -> Result<TerminationRouted, TerminateDelegatedAgentError> {
        let attempt = match self
            .routing
            .shutdown_child(&child, ShutdownReason::SelectedTermination)
            .await
        {
            Ok(()) => ProtocolAttempt::Acknowledged,
            Err(error) => ProtocolAttempt::Negative(error.to_string()),
        };
        let Some(owner) = &self.owner else {
            // A bare route observes nothing beyond the acknowledgement.
            return match attempt {
                ProtocolAttempt::Acknowledged => Ok(TerminationRouted::ShutdownRequested {
                    child,
                    result: None,
                }),
                ProtocolAttempt::Negative(detail) => {
                    self.release_target(&child, claimed);
                    Err(TerminateDelegatedAgentError::ChildUnreachable {
                        child: child.uuid,
                        detail,
                    })
                }
            };
        };
        let result = match self.conclude(owner, &child, attempt).await {
            Ok(result) => result,
            Err(Refusal::NothingDispatched(error)) => {
                self.release_target(&child, claimed);
                return Err(error);
            }
            // Effects reached the child: the claim stays so its eventual
            // exit is compensated as this termination.
            Err(Refusal::AfterEffects(error)) => return Err(error),
        };
        self.compensate(owner, &child).await?;
        Ok(TerminationRouted::ShutdownRequested {
            child,
            result: Some(result),
        })
    }

    /// Observe the child's end: the owned-handle fallback if this harness
    /// retains its process, otherwise the compensation observation alone.
    async fn conclude(
        &self,
        owner: &OwnerConclusionPorts,
        child: &DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
    ) -> Result<TerminationResult, Refusal> {
        let negative = match &attempt {
            ProtocolAttempt::Acknowledged => None,
            ProtocolAttempt::Negative(detail) => Some(detail.clone()),
        };
        match owner
            .termination
            .conclude(child, attempt, ConclusionBudget::Standard)
            .await
        {
            TerminationConclusion::NoRetainedHandle => match negative {
                // Nothing reached the child. Its edge may have failed
                // because it was already gone — its reaper or monitor
                // claimed the terminal effects before the protocol
                // answered — which is an exited child, not a failure.
                Some(_) if owner.registry.terminal_claimed(child) => {
                    Ok(TerminationResult::AlreadyExited)
                }
                Some(detail) => Err(Refusal::NothingDispatched(
                    TerminateDelegatedAgentError::ChildUnreachable {
                        child: child.uuid.clone(),
                        detail,
                    },
                )),
                // Acknowledged but not owned (a script or container
                // member): the only observation is its row's compensation.
                None => match owner.registry.await_compensated(child).await {
                    CompensationObservation::Compensated => Ok(TerminationResult::Graceful),
                    CompensationObservation::TimedOut => Err(Refusal::AfterEffects(
                        TerminateDelegatedAgentError::TerminationFailed {
                            child: child.uuid.clone(),
                            detail: "acknowledged but its exit was not observed within the bound"
                                .into(),
                        },
                    )),
                    CompensationObservation::Unknown => Err(Refusal::NothingDispatched(
                        TerminateDelegatedAgentError::Rejected(
                            TerminationRouteError::UnknownTarget(child.uuid.clone()),
                        ),
                    )),
                },
            },
            TerminationConclusion::AlreadyExited => Ok(TerminationResult::AlreadyExited),
            TerminationConclusion::ExitedAfterProtocol => Ok(TerminationResult::Graceful),
            TerminationConclusion::ExitedAfterFallback => Ok(TerminationResult::Fallback),
            TerminationConclusion::StillRunning(detail) => Err(Refusal::AfterEffects(
                TerminateDelegatedAgentError::TerminationFailed {
                    child: child.uuid.clone(),
                    detail,
                },
            )),
        }
    }

    /// The end was observed: claim the terminal effects, or join the path
    /// (reaper, monitor) that already claimed them.
    async fn compensate(
        &self,
        owner: &OwnerConclusionPorts,
        child: &DelegatedAgentIdentity,
    ) -> Result<(), TerminateDelegatedAgentError> {
        match owner.registry.claim_terminal(child) {
            TerminalClaim::Claimed => {
                owner
                    .compensation
                    .compensate(child, TerminationCause::SelectedTermination)
                    .await;
                Ok(())
            }
            TerminalClaim::AlreadyClaimed => match owner.registry.await_compensated(child).await {
                CompensationObservation::Compensated | CompensationObservation::Unknown => Ok(()),
                CompensationObservation::TimedOut => {
                    Err(TerminateDelegatedAgentError::TerminationFailed {
                        child: child.uuid.clone(),
                        detail:
                            "exit observed but its compensation did not finish within the bound"
                                .into(),
                    })
                }
            },
        }
    }
}

/// Why a conclusion did not yield a result, split by whether the claim
/// this edge took may be lifted.
enum Refusal {
    NothingDispatched(TerminateDelegatedAgentError),
    AfterEffects(TerminateDelegatedAgentError),
}

#[cfg(test)]
#[path = "terminate_delegated_agent_tests.rs"]
mod tests;

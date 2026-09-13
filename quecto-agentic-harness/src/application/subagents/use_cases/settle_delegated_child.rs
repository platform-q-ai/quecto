//! Settlement of one direct child by the parent's hand (#1938, #1939).
//!
//! The one ladder every parent-initiated end of a direct child walks: claim
//! the row stopping under the caller's cause, ask the child to shut down
//! over its one edge, conclude it through the owned-handle fallback
//! (protocol first; a signal only after a negative outcome or an exit
//! timeout, and only for a handle this harness owns), and compensate
//! exactly once — or join the path (reaper, monitor, operator kill,
//! rollback) that already ended it. A child this harness holds no process
//! for is asked, given the compensation bound to exit, then compensated
//! *unobserved*: its environment's retained kill is what ends it. A child
//! whose end cannot be settled is reported unsettled with its stopping
//! claim lifted, so ownership is never silently dropped.
//!
//! The fleet teardown settles every direct child through this; an
//! environment kill settles the environment's members through it.
use std::sync::Arc;

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{DelegatedAgentIdentity, ShutdownReason};

use super::super::dto::{FleetChildResult, SettledChild};
use super::super::ports::{
    CompensationObservation, ConclusionBudget, DelegatedAgentRegistry, DirectChildRouting,
    OwnedChildTermination, ProtocolAttempt, StoppingClaimError, TeardownCompensation,
    TerminalClaim, TerminationCause, TerminationConclusion,
};

/// What settling one child established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildSettlement {
    Done(SettledChild),
    /// The child's end could not be observed within the bound; its row
    /// stays live with the stopping claim lifted.
    Unsettled(AgentUuid, String),
    /// The row was gone (uncommitted or unknown) before anything ran.
    Gone(AgentUuid),
}

pub struct SettleDelegatedChildPorts {
    pub registry: Arc<dyn DelegatedAgentRegistry>,
    pub routing: Arc<dyn DirectChildRouting>,
    pub termination: Arc<dyn OwnedChildTermination>,
    pub compensation: Arc<dyn TeardownCompensation>,
}

pub struct SettleDelegatedChild {
    ports: SettleDelegatedChildPorts,
}

impl SettleDelegatedChild {
    pub fn new(ports: SettleDelegatedChildPorts) -> Self {
        Self { ports }
    }

    /// Claim, ask, conclude, compensate — or join whoever already did.
    /// `cause` is the intent recorded on the row (what the compensation
    /// honours); `reason` travels to the child on the wire.
    pub async fn settle(
        &self,
        child: DelegatedAgentIdentity,
        reason: ShutdownReason,
        cause: TerminationCause,
    ) -> ChildSettlement {
        debug_assert!(
            !matches!(cause, TerminationCause::Exit(_)),
            "a settlement is by the parent's hand, never an observed exit"
        );
        match self.ports.registry.claim_stopping(&child, cause) {
            Ok(()) => {}
            Err(StoppingClaimError::AlreadyStopping) => {
                // A kill, rollback or another teardown owns this child: its
                // compensation is the only truthful end to wait for.
                return self.join_child(child, FleetChildResult::Joined).await;
            }
            Err(StoppingClaimError::Unknown | StoppingClaimError::Exited) => {
                return ChildSettlement::Gone(child.uuid);
            }
        }
        let attempt = ProtocolAttempt::from_shutdown_answer(
            self.ports.routing.shutdown_child(&child, reason).await,
        );
        let acknowledged = matches!(attempt, ProtocolAttempt::Acknowledged);
        let result = match self
            .ports
            .termination
            .conclude(&child, attempt, ConclusionBudget::Standard)
            .await
        {
            TerminationConclusion::ExitedAfterProtocol => FleetChildResult::Graceful,
            TerminationConclusion::ExitedAfterFallback => FleetChildResult::Fallback,
            TerminationConclusion::AlreadyExited => FleetChildResult::AlreadyExited,
            TerminationConclusion::StillRunning(detail) => {
                // Even the fallback did not end it: the row keeps its record
                // and the claim is lifted so a later trigger may try again.
                self.ports.registry.release_stopping(&child);
                return ChildSettlement::Unsettled(child.uuid, detail);
            }
            TerminationConclusion::NoRetainedHandle => {
                self.conclude_unowned(&child, acknowledged).await
            }
        };
        self.compensate_or_join(child, result, cause).await
    }

    /// A child this harness holds no process for: its end is observed only
    /// through its row (monitor EOF, reported prune, reaper of an earlier
    /// handle). An acknowledged child gets the bound to exit on its own;
    /// past it — or after a negative attempt — the compensation runs
    /// without an observed exit, which finalizes its environment.
    async fn conclude_unowned(
        &self,
        child: &DelegatedAgentIdentity,
        acknowledged: bool,
    ) -> FleetChildResult {
        if self.ports.registry.terminal_claimed(child) {
            return FleetChildResult::AlreadyExited;
        }
        if !acknowledged {
            return FleetChildResult::Unobserved;
        }
        match self.ports.registry.await_compensated(child).await {
            CompensationObservation::Compensated => FleetChildResult::Graceful,
            CompensationObservation::TimedOut | CompensationObservation::Unknown => {
                FleetChildResult::Unobserved
            }
        }
    }

    async fn compensate_or_join(
        &self,
        child: DelegatedAgentIdentity,
        result: FleetChildResult,
        cause: TerminationCause,
    ) -> ChildSettlement {
        match self.ports.registry.claim_terminal(&child) {
            TerminalClaim::Claimed => {
                self.ports.compensation.compensate(&child, cause).await;
                ChildSettlement::Done(SettledChild { child, result })
            }
            TerminalClaim::AlreadyClaimed => self.join_child(child, result).await,
        }
    }

    async fn join_child(
        &self,
        child: DelegatedAgentIdentity,
        result: FleetChildResult,
    ) -> ChildSettlement {
        match self.ports.registry.await_compensated(&child).await {
            CompensationObservation::Compensated => {
                ChildSettlement::Done(SettledChild { child, result })
            }
            CompensationObservation::Unknown => ChildSettlement::Gone(child.uuid),
            CompensationObservation::TimedOut => ChildSettlement::Unsettled(
                child.uuid,
                "a termination already in flight did not settle within the bound".into(),
            ),
        }
    }
}

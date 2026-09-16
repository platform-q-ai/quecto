//! The member-shutdown adapter maps the per-child settlement onto the
//! environments port: resolution refusals and settlements each land on the
//! truthful side of the report, in membership order.
use std::sync::{Arc, Mutex};

use super::DelegatedMemberShutdown;
use crate::application::environments::ports::{EnvironmentMemberShutdown, MemberShutdownResult};
use crate::application::subagents::ports::{
    ChildRoutingError, Compensated, CompensationObservation, ConclusionBudget,
    DelegatedAgentRegistry, DirectChildRouting, OwnedChildTermination, PortFuture, ProtocolAttempt,
    ResolutionError, StoppingClaimError, TeardownCompensation, TerminalClaim, TerminationCause,
    TerminationConclusion, TerminationResult,
};
use crate::application::subagents::use_cases::{SettleDelegatedChild, SettleDelegatedChildPorts};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, RoutingDepth, ShutdownReason,
};

/// Rows keyed by uuid with a scripted resolution, protocol answer and
/// conclusion; records the causes each claim and compensation carried.
#[derive(Default)]
struct Fakes {
    resolve: Mutex<std::collections::HashMap<String, Result<u64, ResolutionError>>>,
    stopping: Mutex<Vec<(String, TerminationCause)>>,
    already_stopping: Mutex<Vec<String>>,
    unreachable: Mutex<Vec<String>>,
    conclusions: Mutex<std::collections::HashMap<String, TerminationConclusion>>,
    compensated: Mutex<Vec<(String, TerminationCause)>>,
    asked: Mutex<Vec<(String, ShutdownReason)>>,
}

impl DelegatedAgentRegistry for Fakes {
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError> {
        match self.resolve.lock().unwrap().get(reference) {
            Some(Ok(generation)) => Ok(DelegatedAgentIdentity::new(
                reference,
                LaunchGeneration::new(*generation),
            )),
            Some(Err(error)) => Err(error.clone()),
            None => Err(ResolutionError::Unknown),
        }
    }
    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError> {
        if self
            .already_stopping
            .lock()
            .unwrap()
            .contains(&target.uuid.as_str().to_owned())
        {
            return Err(StoppingClaimError::AlreadyStopping);
        }
        self.stopping
            .lock()
            .unwrap()
            .push((target.uuid.as_str().to_owned(), cause));
        Ok(())
    }
    fn release_stopping(&self, _: &DelegatedAgentIdentity) {}
    fn retain_stopping(&self, _: &DelegatedAgentIdentity) {}
    fn claim_terminal(&self, _: &DelegatedAgentIdentity) -> TerminalClaim {
        TerminalClaim::Claimed
    }
    fn holds_process(&self, _: &DelegatedAgentIdentity) -> bool {
        false
    }
    fn terminal_claimed(&self, _: &DelegatedAgentIdentity) -> bool {
        false
    }
    fn await_compensated<'a>(
        &'a self,
        _: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation> {
        Box::pin(async { CompensationObservation::TimedOut })
    }
}

impl DirectChildRouting for Fakes {
    fn shutdown_child<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((child.uuid.as_str().to_owned(), reason));
            if self
                .unreachable
                .lock()
                .unwrap()
                .contains(&child.uuid.as_str().to_owned())
            {
                Err(ChildRoutingError::Unreachable("gone".into()))
            } else {
                Ok(())
            }
        })
    }
    fn forward_termination<'a>(
        &'a self,
        _: &'a DelegatedAgentIdentity,
        _: &'a DelegatedAgentIdentity,
        _: RoutingDepth,
    ) -> PortFuture<'a, Result<Option<TerminationResult>, ChildRoutingError>> {
        Box::pin(async { panic!("an environment member is never forwarded") })
    }
}

impl OwnedChildTermination for Fakes {
    fn conclude<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        _: ProtocolAttempt,
        _: ConclusionBudget,
    ) -> PortFuture<'a, TerminationConclusion> {
        Box::pin(async move {
            self.conclusions
                .lock()
                .unwrap()
                .get(child.uuid.as_str())
                .cloned()
                .unwrap_or(TerminationConclusion::NoRetainedHandle)
        })
    }
}

impl TeardownCompensation for Fakes {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated> {
        Box::pin(async move {
            self.compensated
                .lock()
                .unwrap()
                .push((target.uuid.as_str().to_owned(), cause));
            Compensated {
                removed: vec![AgentUuid::new(target.uuid.as_str())],
            }
        })
    }
    fn prune_terminal_rows(&self) -> PortFuture<'_, Vec<AgentUuid>> {
        Box::pin(async { Vec::new() })
    }
}

fn adapter(fakes: &Arc<Fakes>) -> DelegatedMemberShutdown {
    DelegatedMemberShutdown::new(
        fakes.clone(),
        Arc::new(SettleDelegatedChild::new(SettleDelegatedChildPorts {
            registry: fakes.clone(),
            routing: fakes.clone(),
            termination: fakes.clone(),
            compensation: fakes.clone(),
        })),
    )
}

#[tokio::test]
async fn each_member_is_settled_under_the_environment_kill_cause_in_order() {
    let fakes = Arc::new(Fakes::default());
    {
        let mut resolve = fakes.resolve.lock().unwrap();
        resolve.insert("owned".into(), Ok(1));
        resolve.insert("container".into(), Ok(2));
        resolve.insert("dead-edge".into(), Ok(3));
        resolve.insert("gone".into(), Err(ResolutionError::Exited));
        resolve.insert("fixture".into(), Err(ResolutionError::NotDelegated));
    }
    fakes
        .conclusions
        .lock()
        .unwrap()
        .insert("owned".into(), TerminationConclusion::ExitedAfterProtocol);
    fakes.unreachable.lock().unwrap().push("dead-edge".into());
    let members: Vec<String> = [
        "owned",
        "container",
        "dead-edge",
        "gone",
        "fixture",
        "never",
    ]
    .iter()
    .map(|m| m.to_string())
    .collect();
    let report = adapter(&fakes).shutdown_members(&members).await;
    let settled: Vec<(&str, MemberShutdownResult)> = report
        .settled
        .iter()
        .map(|m| (m.member.as_str(), m.result))
        .collect();
    assert_eq!(
        settled,
        [
            ("owned", MemberShutdownResult::Graceful),
            ("container", MemberShutdownResult::Unobserved),
            ("dead-edge", MemberShutdownResult::Unobserved),
            ("gone", MemberShutdownResult::AlreadyExited),
            ("never", MemberShutdownResult::AlreadyExited),
        ]
    );
    assert_eq!(report.unsettled.len(), 1);
    assert_eq!(report.unsettled[0].member, "fixture");
    assert!(
        report.unsettled[0].detail.contains("not a delegated agent"),
        "{report:?}"
    );
    let asked = fakes.asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        [
            ("owned".to_string(), ShutdownReason::OperatorRequest),
            ("container".to_string(), ShutdownReason::OperatorRequest),
            ("dead-edge".to_string(), ShutdownReason::OperatorRequest),
        ]
    );
    for (_, cause) in fakes.stopping.lock().unwrap().iter() {
        assert_eq!(*cause, TerminationCause::EnvironmentKill);
    }
    let compensated = fakes.compensated.lock().unwrap().clone();
    assert_eq!(compensated.len(), 3, "{compensated:?}");
    assert!(
        compensated
            .iter()
            .all(|(_, cause)| *cause == TerminationCause::EnvironmentKill)
    );
}

#[tokio::test]
async fn a_member_owned_by_another_termination_that_never_settles_is_unsettled() {
    let fakes = Arc::new(Fakes::default());
    fakes.resolve.lock().unwrap().insert("busy".into(), Ok(1));
    fakes.already_stopping.lock().unwrap().push("busy".into());
    let report = adapter(&fakes)
        .shutdown_members(&["busy".to_string()])
        .await;
    assert!(report.settled.is_empty());
    assert_eq!(report.unsettled.len(), 1);
    assert!(
        report.unsettled[0].detail.contains("in flight"),
        "{report:?}"
    );
    assert!(fakes.asked.lock().unwrap().is_empty());
    assert!(fakes.compensated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_still_running_owned_member_is_unsettled_and_nothing_is_compensated() {
    let fakes = Arc::new(Fakes::default());
    fakes.resolve.lock().unwrap().insert("stuck".into(), Ok(1));
    fakes.conclusions.lock().unwrap().insert(
        "stuck".into(),
        TerminationConclusion::StillRunning("no exit within budget".into()),
    );
    let report = adapter(&fakes)
        .shutdown_members(&["stuck".to_string()])
        .await;
    assert_eq!(report.unsettled.len(), 1);
    assert_eq!(report.unsettled[0].detail, "no exit within budget");
    assert!(fakes.compensated.lock().unwrap().is_empty());
}

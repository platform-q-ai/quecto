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

/// #1953 review (3): an empty [`super::MemberShutdownSlot`] — no
/// composition installed an owner — reports every member unsettled, so
/// `kill_container` refuses before the retained kill and leaves the
/// environment retryable; once an owner is installed, the slot delegates
/// to it, and a second install is ignored.
mod slot {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::MemberShutdownSlot;
    use crate::application::environments::ports::{
        EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport,
        MemberShutdownResult, PortFuture, SettledMember,
    };
    use crate::application::environments::use_cases::{KillEnvironment, KillEnvironmentError};
    use crate::domain::environment_registry::{
        EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
        mint_environment_uuid,
    };

    #[derive(Default)]
    struct CountingCommands {
        kills: AtomicUsize,
    }

    impl EnvironmentProcessCommands for CountingCommands {
        fn run_retained_inspect<'a>(
            &'a self,
            _: &'a str,
            _: &'a [String],
        ) -> PortFuture<'a, Result<serde_json::Value, String>> {
            Box::pin(async { panic!("kill_container never inspects") })
        }
        fn run_retained_kill<'a>(
            &'a self,
            _: &'a str,
            _: &'a [String],
        ) -> PortFuture<'a, Result<(), String>> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
        fn run_retained_cleanup<'a>(&'a self, _: &'a str, _: &'a [String]) -> PortFuture<'a, ()> {
            Box::pin(async { panic!("kill_container never runs cleanup") })
        }
    }

    struct EveryMemberSettled;

    impl EnvironmentMemberShutdown for EveryMemberSettled {
        fn shutdown_members<'a>(
            &'a self,
            members: &'a [String],
        ) -> PortFuture<'a, MemberShutdownReport> {
            Box::pin(async move {
                MemberShutdownReport {
                    settled: members
                        .iter()
                        .map(|member| SettledMember {
                            member: member.clone(),
                            result: MemberShutdownResult::Graceful,
                        })
                        .collect(),
                    unsettled: Vec::new(),
                }
            })
        }
    }

    fn environment_with_member() -> (EnvironmentRegistry, String) {
        let environments = EnvironmentRegistry::new();
        let env_ref = environments.mint_ref();
        environments.commit(EnvironmentRecord {
            environment_ref: env_ref.clone(),
            environment_id: "runtime-slot".into(),
            environment_uuid: mint_environment_uuid(),
            name: None,
            workspace_path: std::path::PathBuf::from("/workspace"),
            repository: String::new(),
            script_name: "default".into(),
            retained_exec_argv: vec![],
            retained_kill_argv: vec!["kill.sh".into()],
            retained_cleanup_argv: vec![],
            retained_inspect_argv: vec![],
            members: vec![],
            status: EnvironmentStatus::Running,
            metadata: serde_json::json!({}),
            last_error: None,
        });
        environments.add_member(&env_ref, "member-1").unwrap();
        (environments, env_ref)
    }

    #[tokio::test]
    async fn an_empty_slot_reports_every_member_unsettled_and_withholds_the_retained_kill() {
        let slot = MemberShutdownSlot::default();
        assert!(slot.get().is_none());
        let report = slot.shutdown_members(&["m1".into(), "m2".into()]).await;
        assert!(report.settled.is_empty());
        assert_eq!(
            report
                .unsettled
                .iter()
                .map(|m| m.member.as_str())
                .collect::<Vec<_>>(),
            ["m1", "m2"]
        );
        assert!(
            report.unsettled[0]
                .detail
                .contains("no member shutdown is composed"),
            "{report:?}"
        );

        let (environments, env_ref) = environment_with_member();
        let commands = Arc::new(CountingCommands::default());
        let kill = KillEnvironment::new(
            environments.clone(),
            Arc::new(slot.clone()),
            commands.clone(),
        );
        let error = kill
            .kill_container(&EnvironmentTarget::Ref(env_ref.clone()))
            .await
            .unwrap_err();
        assert!(
            matches!(error, KillEnvironmentError::MembersUnsettled { .. }),
            "{error:?}"
        );
        assert_eq!(commands.kills.load(Ordering::SeqCst), 0, "no retained kill");
        let record = environments.get(&env_ref).unwrap();
        assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
        assert_eq!(record.members, ["member-1"], "membership kept for a retry");

        // An installed owner takes over; the second install is ignored.
        assert!(slot.install(Arc::new(EveryMemberSettled)));
        assert!(!slot.install(Arc::new(EveryMemberSettled)));
        let killed = kill
            .kill_container(&EnvironmentTarget::Ref(env_ref.clone()))
            .await
            .unwrap();
        assert_eq!(killed.members.settled.len(), 1);
        assert_eq!(commands.kills.load(Ordering::SeqCst), 1);
        assert_eq!(
            environments.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
}

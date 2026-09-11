//! #1924: a swarm's environment is retained after every end of its run —
//! orderly, closed, cancelled or by loss of its coordinator — instead of
//! being killed with the final member; only an explicit kill closes it.

use std::sync::{Arc, Mutex};

use super::environment_finalization_tests::{ScriptCall, block_on, committed_env};
use crate::domain::environment_finalization::{
    CoordinatorLoss, EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, HostedSwarmRun,
    MemberFinalizeMode, SwarmRunObservation, retains_environment, retention_reason,
};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::RunStatus;

/// A port whose environment hosts the configured swarm run (or none) and
/// records every lost-coordinator record it is asked to make, answering
/// like the store: an ended run is left alone, any other run is paused
/// holding `failed`.
struct HostedRunPort {
    observed: SwarmRunObservation,
    loss_error: Option<String>,
    losses: Mutex<Vec<String>>,
    kills: Mutex<Vec<ScriptCall>>,
    cleanups: Mutex<Vec<ScriptCall>>,
}

impl HostedRunPort {
    fn hosting(hosted: Option<HostedSwarmRun>) -> Self {
        Self::observing(hosted.map_or(SwarmRunObservation::NoStore, SwarmRunObservation::Run))
    }

    fn observing(observed: SwarmRunObservation) -> Self {
        Self {
            observed,
            loss_error: None,
            losses: Mutex::new(Vec::new()),
            kills: Mutex::new(Vec::new()),
            cleanups: Mutex::new(Vec::new()),
        }
    }
}

impl EnvironmentFinalizationPort for HostedRunPort {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, SwarmRunObservation> {
        Box::pin(async move { self.observed.clone() })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> LaunchFuture<'a, Result<CoordinatorLoss, String>> {
        Box::pin(async move {
            if let Some(error) = &self.loss_error {
                return Err(error.clone());
            }
            if hosted.ended() {
                return Ok(CoordinatorLoss {
                    run: hosted.clone(),
                    lost: false,
                });
            }
            self.losses.lock().unwrap().push(hosted.coordinator.clone());
            Ok(CoordinatorLoss {
                run: HostedSwarmRun {
                    status: RunStatus::Paused,
                    outcome: Some(RunStatus::Failed),
                    ..hosted.clone()
                },
                lost: true,
            })
        })
    }

    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.kills.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            Ok(())
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()> {
        Box::pin(async move {
            self.cleanups.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
        })
    }
}

fn running_swarm() -> HostedSwarmRun {
    HostedSwarmRun {
        status: RunStatus::Running,
        outcome: None,
        coordinator: "member-42".to_string(),
        deadline: 4_102_444_800.0,
    }
}

fn with_status(status: RunStatus, outcome: Option<RunStatus>) -> HostedSwarmRun {
    HostedSwarmRun {
        status,
        outcome,
        ..running_swarm()
    }
}

fn finalize(port: Arc<HostedRunPort>, mode: MemberFinalizeMode) -> (EnvironmentRegistry, String) {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port);
    block_on(use_case.finalize_member(&env_ref, "coordinator-uuid", None, mode));
    (registry, env_ref)
}

fn reason(registry: &EnvironmentRegistry, env_ref: &str) -> String {
    registry.get(env_ref).unwrap().metadata["retained"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn retention_policy_holds_for_every_created_run_on_exit_and_parent_kill() {
    let run = SwarmRunObservation::Run;
    let unreadable = SwarmRunObservation::Unreadable("locked".to_string());
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        for status in [
            RunStatus::Running,
            RunStatus::Paused,
            RunStatus::Succeeded,
            RunStatus::Blocked,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::BudgetExhausted,
        ] {
            assert!(
                retains_environment(mode, &run(with_status(status, None))),
                "{mode:?} on a created {status:?} run retains"
            );
        }
        assert!(
            !retains_environment(mode, &run(placeholder.clone())),
            "{mode:?}: the bootstrap placeholder is no swarm"
        );
        assert!(!retains_environment(mode, &SwarmRunObservation::NoStore));
        assert!(
            retains_environment(mode, &unreadable),
            "{mode:?}: a store that exists but cannot be read is never proof of no run"
        );
    }
    for mode in [
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        assert!(
            !retains_environment(mode, &run(running_swarm())),
            "{mode:?} has nothing to inspect and keeps its cleanup"
        );
        assert!(!retains_environment(mode, &unreadable));
    }
}

#[test]
fn retention_reasons_name_how_the_run_ended() {
    let exit = MemberFinalizeMode::Exit;
    let text = |run| retention_reason(exit, &SwarmRunObservation::Run(run));
    assert!(text(with_status(RunStatus::Succeeded, None)).starts_with("run closed: succeeded;"));
    assert!(text(with_status(RunStatus::Failed, None)).starts_with("run closed: failed;"));
    assert!(text(with_status(RunStatus::Cancelled, None)).starts_with("run ended: cancelled;"));
    assert!(
        text(with_status(
            RunStatus::Paused,
            Some(RunStatus::BudgetExhausted)
        ))
        .starts_with("run ended: budget-exhausted;")
    );
    let killed = retention_reason(
        MemberFinalizeMode::ParentKill,
        &SwarmRunObservation::Run(with_status(RunStatus::Paused, Some(RunStatus::Succeeded))),
    );
    assert!(
        killed.starts_with("coordinator killed by supervisor; run paused holding succeeded;"),
        "{killed}"
    );
    let killed_running = retention_reason(
        MemberFinalizeMode::ParentKill,
        &SwarmRunObservation::Run(running_swarm()),
    );
    assert!(
        killed_running.starts_with("coordinator killed by supervisor; run running;"),
        "{killed_running}"
    );
    for reason in [
        text(running_swarm()),
        killed,
        retention_reason(exit, &SwarmRunObservation::Unreadable("x".into())),
    ] {
        assert!(reason.ends_with("kill_container to remove"), "{reason}");
    }
}

#[test]
fn coordinator_exit_with_running_swarm_withholds_kill_and_records_the_loss() {
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);

    assert!(port.kills.lock().unwrap().is_empty(), "no retained kill");
    assert!(
        port.cleanups.lock().unwrap().is_empty(),
        "no retained cleanup"
    );
    assert_eq!(
        port.losses.lock().unwrap().as_slice(),
        &["member-42".to_string()],
        "the #1729 lost-harness rule is applied to the coordinator"
    );
    let record = registry.get(&env_ref).expect("record kept");
    assert_eq!(record.status, EnvironmentStatus::Retained);
    assert_eq!(record.status_label(), "retained");
    assert!(record.members.is_empty(), "the member is still removed");
    let reason = reason(&registry, &env_ref);
    assert!(
        reason.contains("'member-42' lost its connection"),
        "{reason}"
    );
    assert!(
        reason.contains("run paused holding failed"),
        "the reason reflects the state the store reports back: {reason}"
    );
    assert!(reason.contains("kill_container"), "{reason}");
    // Only an explicit kill_container may now run the retained kill.
    assert!(registry.begin_kill(&env_ref).is_ok());
}

#[test]
fn coordinator_exit_from_a_plain_pause_is_a_loss() {
    let port = Arc::new(HostedRunPort::hosting(Some(with_status(
        RunStatus::Paused,
        None,
    ))));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    assert_eq!(
        port.losses.lock().unwrap().as_slice(),
        &["member-42".to_string()]
    );
    assert!(reason(&registry, &env_ref).contains("lost its connection"));
}

#[test]
fn coordinator_exit_after_an_orderly_end_retains_without_quarantine() {
    // The user's decision: every swarm end keeps its container. A run
    // already paused holding an outcome, closed, or cancelled is not a loss.
    let cases = [
        (
            with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
            "run ended: succeeded",
        ),
        (
            with_status(RunStatus::Paused, Some(RunStatus::Blocked)),
            "run ended: blocked",
        ),
        (
            with_status(RunStatus::Paused, Some(RunStatus::Failed)),
            "run ended: failed",
        ),
        (
            with_status(RunStatus::Paused, Some(RunStatus::BudgetExhausted)),
            "run ended: budget-exhausted",
        ),
        (
            with_status(RunStatus::Succeeded, None),
            "run closed: succeeded",
        ),
        (with_status(RunStatus::Blocked, None), "run closed: blocked"),
        (
            with_status(RunStatus::Cancelled, None),
            "run ended: cancelled",
        ),
    ];
    for (hosted, expected) in cases {
        assert!(hosted.ended(), "{hosted:?}");
        let port = Arc::new(HostedRunPort::hosting(Some(hosted.clone())));
        let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
        assert!(port.kills.lock().unwrap().is_empty(), "{hosted:?}");
        assert!(
            port.losses.lock().unwrap().is_empty(),
            "an orderly end never quarantines the coordinator ({hosted:?})"
        );
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Retained
        );
        let reason = reason(&registry, &env_ref);
        assert!(reason.starts_with(expected), "{hosted:?}: {reason}");
        assert!(!reason.contains("lost"), "{reason}");
    }
}

#[test]
fn a_run_that_ends_between_observation_and_the_loss_record_is_reported_orderly() {
    // The port answers from ONE store operation: the run the observer saw as
    // running had expired (budget-exhausted) by the time the loss was
    // recorded, so nothing is quarantined and the reason says so.
    struct ExpiringPort(HostedRunPort);
    impl EnvironmentFinalizationPort for ExpiringPort {
        fn observe_hosted_swarm_run<'a>(
            &'a self,
            record: &'a EnvironmentRecord,
        ) -> LaunchFuture<'a, SwarmRunObservation> {
            self.0.observe_hosted_swarm_run(record)
        }
        fn record_lost_coordinator<'a>(
            &'a self,
            _record: &'a EnvironmentRecord,
            hosted: &'a HostedSwarmRun,
        ) -> LaunchFuture<'a, Result<CoordinatorLoss, String>> {
            assert_eq!(hosted.status, RunStatus::Running, "observed as running");
            Box::pin(async move {
                Ok(CoordinatorLoss {
                    run: with_status(RunStatus::Paused, Some(RunStatus::BudgetExhausted)),
                    lost: false,
                })
            })
        }
        fn run_retained_inspect<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> LaunchFuture<'a, Result<serde_json::Value, String>> {
            self.0.run_retained_inspect(environment_id, argv)
        }
        fn run_retained_kill<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> LaunchFuture<'a, Result<(), String>> {
            self.0.run_retained_kill(environment_id, argv)
        }
        fn run_retained_cleanup<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> LaunchFuture<'a, ()> {
            self.0.run_retained_cleanup(environment_id, argv)
        }
    }
    let port = Arc::new(ExpiringPort(HostedRunPort::hosting(Some(running_swarm()))));
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());
    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));
    assert!(port.0.kills.lock().unwrap().is_empty());
    let reason = reason(&registry, &env_ref);
    assert!(
        reason.starts_with("run ended: budget-exhausted"),
        "{reason}"
    );
}

#[test]
fn coordinator_exit_still_retains_when_the_store_refuses_the_loss_record() {
    let mut port = HostedRunPort::hosting(Some(running_swarm()));
    port.loss_error = Some("store locked".to_string());
    let port = Arc::new(port);
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    assert!(port.kills.lock().unwrap().is_empty());
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained
    );
    assert!(reason(&registry, &env_ref).contains("store locked"));
}

#[test]
fn supervisor_kill_of_the_coordinator_retains_without_a_loss_record() {
    // agent_cmd kill of the member, or the master's own shutdown: the box
    // the master was told it could inspect must survive, and a deliberate
    // kill is not a lost harness.
    for hosted in [
        running_swarm(),
        with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
        with_status(RunStatus::Succeeded, None),
    ] {
        let port = Arc::new(HostedRunPort::hosting(Some(hosted.clone())));
        let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::ParentKill);
        assert!(port.kills.lock().unwrap().is_empty(), "{hosted:?}");
        assert!(port.losses.lock().unwrap().is_empty(), "{hosted:?}");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Retained
        );
        let reason = reason(&registry, &env_ref);
        assert!(
            reason.starts_with("coordinator killed by supervisor; run "),
            "{reason}"
        );
        assert!(registry.begin_kill(&env_ref).is_ok());
    }
}

#[test]
fn plain_member_exit_without_a_swarm_keeps_the_final_member_kill() {
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        let port = Arc::new(HostedRunPort::hosting(None));
        let (registry, env_ref) = finalize(port.clone(), mode);
        assert_eq!(port.kills.lock().unwrap().len(), 1, "{mode:?}");
        assert!(port.losses.lock().unwrap().is_empty());
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped
        );
    }
}

#[test]
fn exit_from_a_container_at_its_bootstrap_placeholder_keeps_the_final_member_kill() {
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    let port = Arc::new(HostedRunPort::hosting(Some(placeholder)));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    assert_eq!(port.kills.lock().unwrap().len(), 1);
    assert!(port.losses.lock().unwrap().is_empty());
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn launch_rollback_with_a_running_swarm_keeps_the_retained_cleanup() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a"]);
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());
    block_on(use_case.finalize_member(
        &env_ref,
        "agent-a",
        None,
        MemberFinalizeMode::LaunchRollbackOwned,
    ));
    assert_eq!(port.cleanups.lock().unwrap().len(), 1);
    assert!(port.kills.lock().unwrap().is_empty());
    assert!(port.losses.lock().unwrap().is_empty());
    assert!(
        registry.get(&env_ref).is_none(),
        "owned rollback discards the record"
    );
}

#[test]
fn a_rolled_back_join_into_a_retained_environment_leaves_it_retained() {
    // A join into a retained environment cannot activate (the store's run is
    // paused), so its launch rolls back; that rollback must neither run a
    // kill nor "complete" one, or the container leaks behind a Stopped record.
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    registry.add_member(&env_ref, "joiner-uuid").unwrap();
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained,
        "a join does not revive a retained environment"
    );
    block_on(use_case.finalize_member(
        &env_ref,
        "joiner-uuid",
        None,
        MemberFinalizeMode::LaunchRollback,
    ));

    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    assert!(record.members.is_empty());
    assert!(port.kills.lock().unwrap().is_empty());
    assert!(port.cleanups.lock().unwrap().is_empty());
    assert!(
        registry.begin_kill(&env_ref).is_ok(),
        "kill_container still closes the retained environment"
    );
}

#[test]
fn a_joiner_exit_from_a_retained_environment_leaves_it_retained() {
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());
    registry.add_member(&env_ref, "joiner-uuid").unwrap();
    block_on(use_case.finalize_member(&env_ref, "joiner-uuid", None, MemberFinalizeMode::Exit));
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained
    );
    assert!(port.kills.lock().unwrap().is_empty());
    assert!(registry.begin_kill(&env_ref).is_ok());
}

#[test]
fn coordinator_exit_with_an_unreadable_store_retains_without_a_loss_record() {
    let port = Arc::new(HostedRunPort::observing(SwarmRunObservation::Unreadable(
        "coordination store unavailable or contended".to_string(),
    )));
    let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::Exit);
    assert!(port.kills.lock().unwrap().is_empty());
    assert!(
        port.losses.lock().unwrap().is_empty(),
        "no run was observed, so nothing is quarantined"
    );
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained
    );
    assert!(reason(&registry, &env_ref).contains("contended"));
}

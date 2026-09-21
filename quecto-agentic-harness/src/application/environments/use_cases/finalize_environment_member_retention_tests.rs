//! #1924, #2070: a swarm's environment lives as long as the swarm. A run its
//! owner has not closed — running, paused, its coordinator lost, cancelled
//! by the coordinator itself — keeps its box when the final member goes. A
//! run the supervisor closed into its outcome, or the owner's explicit
//! teardown, gives the box up: the retained kill runs.

use std::sync::{Arc, Mutex};

use super::finalize_environment_member_tests::{ScriptCall, block_on, committed_env};
use crate::application::environments::ports::{
    EnvironmentProcessCommands, HostedSwarmRunObservation, PortFuture,
};
use crate::application::environments::use_cases::FinalizeEnvironmentMember;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::environment_retention::{
    CoordinatorLoss, HostedSwarmRun, MemberFinalizeMode, SwarmRunObservation,
};
use crate::domain::swarm::RunStatus;

/// One fake standing in for both ports: the hosted run it observes and the
/// scripts it records.
fn use_case<P: HostedSwarmRunObservation + EnvironmentProcessCommands + 'static>(
    registry: &EnvironmentRegistry,
    port: Arc<P>,
) -> FinalizeEnvironmentMember {
    FinalizeEnvironmentMember::new(registry.clone(), port.clone(), port)
}

/// A port whose environment hosts the configured swarm run (or none) and
/// records every lost-coordinator record it is asked to make, answering
/// like the store: an ended run is left alone, any other run is paused
/// holding `failed`.
struct HostedRunPort {
    observed: SwarmRunObservation,
    loss_error: Option<String>,
    observations: Mutex<usize>,
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
            observations: Mutex::new(0),
            losses: Mutex::new(Vec::new()),
            kills: Mutex::new(Vec::new()),
            cleanups: Mutex::new(Vec::new()),
        }
    }
}

impl HostedSwarmRunObservation for HostedRunPort {
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation> {
        *self.observations.lock().unwrap() += 1;
        Box::pin(async move { self.observed.clone() })
    }

    fn record_lost_coordinator<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
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
}

impl EnvironmentProcessCommands for HostedRunPort {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
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
    ) -> PortFuture<'a, ()> {
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
        id: "run-42".to_string(),
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
    let use_case = use_case(&registry, port);
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
fn coordinator_exit_from_a_run_holding_an_outcome_retains_without_quarantine() {
    // Paused holding an outcome is an orderly end the owner has not closed
    // yet: not a loss, and not over — the box stays (#2070).
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
fn a_run_its_owner_closed_gives_up_its_container() {
    // #2070: the container lives as long as the swarm and no longer. Only
    // the supervisor outside the swarm can close a run into its outcome.
    for status in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::BudgetExhausted,
    ] {
        for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
            let port = Arc::new(HostedRunPort::hosting(Some(with_status(status, None))));
            let (registry, env_ref) = finalize(port.clone(), mode);
            assert_eq!(
                port.kills.lock().unwrap().len(),
                1,
                "{status:?}/{mode:?}: the retained kill runs exactly once"
            );
            assert!(
                port.losses.lock().unwrap().is_empty(),
                "{status:?}/{mode:?}: an ended run is no loss"
            );
            let record = registry.get(&env_ref).expect("record kept until collected");
            assert_eq!(
                record.status,
                EnvironmentStatus::Stopped,
                "{status:?}/{mode:?}"
            );
            assert!(
                record.metadata.get("retained").is_none(),
                "{status:?}/{mode:?}"
            );
        }
    }
}

#[test]
fn a_run_its_coordinator_cancelled_keeps_its_container() {
    // `cancelled` is written by the coordinator agent itself, never by the
    // owner: an agent must not be able to destroy its own box (#2070).
    for mode in [MemberFinalizeMode::Exit, MemberFinalizeMode::ParentKill] {
        let port = Arc::new(HostedRunPort::hosting(Some(with_status(
            RunStatus::Cancelled,
            None,
        ))));
        let (registry, env_ref) = finalize(port.clone(), mode);
        assert!(port.kills.lock().unwrap().is_empty(), "{mode:?}");
        assert!(port.losses.lock().unwrap().is_empty(), "{mode:?}: no loss");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Retained,
            "{mode:?}"
        );
    }
    let port = Arc::new(HostedRunPort::hosting(Some(with_status(
        RunStatus::Cancelled,
        None,
    ))));
    let (registry, env_ref) = finalize(port, MemberFinalizeMode::Exit);
    let reason = reason(&registry, &env_ref);
    assert!(reason.starts_with("run ended: cancelled"), "{reason}");
}

#[test]
fn the_owners_explicit_teardown_ends_a_swarm_whatever_its_state() {
    // #2070: delete-all or a session transition is the owner saying "done" —
    // a running run, a paused one, even a store that cannot be read: nothing
    // is kept, nothing is recorded as lost, and the store is not even read.
    for observed in [
        SwarmRunObservation::Run(running_swarm()),
        SwarmRunObservation::Run(with_status(RunStatus::Paused, Some(RunStatus::Failed))),
        SwarmRunObservation::Unreadable("database is locked".to_string()),
    ] {
        let port = Arc::new(HostedRunPort::observing(observed.clone()));
        let (registry, env_ref) = finalize(port.clone(), MemberFinalizeMode::OwnerTeardown);
        assert_eq!(
            *port.observations.lock().unwrap(),
            0,
            "the owner's word needs no look at the run ({observed:?})"
        );
        assert_eq!(port.kills.lock().unwrap().len(), 1, "{observed:?}");
        assert!(port.cleanups.lock().unwrap().is_empty(), "{observed:?}");
        assert!(port.losses.lock().unwrap().is_empty(), "{observed:?}");
        assert_eq!(
            registry.get(&env_ref).unwrap().status,
            EnvironmentStatus::Stopped,
            "{observed:?}"
        );
    }
}

#[test]
fn a_run_that_ends_between_observation_and_the_loss_record_follows_how_it_ended() {
    // The port answers from ONE store operation: the run the observer saw as
    // running had changed by the time the loss was recorded. Expired
    // (paused holding budget-exhausted): nothing is quarantined, the box is
    // kept and the reason says so. Closed by its owner in between (#2070):
    // the swarm is over, so the kill runs after all.
    struct ExpiringPort(HostedRunPort, HostedSwarmRun);
    impl HostedSwarmRunObservation for ExpiringPort {
        fn observe_hosted_swarm_run<'a>(
            &'a self,
            record: &'a EnvironmentRecord,
        ) -> PortFuture<'a, SwarmRunObservation> {
            self.0.observe_hosted_swarm_run(record)
        }
        fn record_lost_coordinator<'a>(
            &'a self,
            _record: &'a EnvironmentRecord,
            hosted: &'a HostedSwarmRun,
        ) -> PortFuture<'a, Result<CoordinatorLoss, String>> {
            assert_eq!(hosted.status, RunStatus::Running, "observed as running");
            Box::pin(async move {
                Ok(CoordinatorLoss {
                    run: self.1.clone(),
                    lost: false,
                })
            })
        }
    }

    impl EnvironmentProcessCommands for ExpiringPort {
        fn run_retained_inspect<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> PortFuture<'a, Result<serde_json::Value, String>> {
            self.0.run_retained_inspect(environment_id, argv)
        }
        fn run_retained_kill<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> PortFuture<'a, Result<(), String>> {
            self.0.run_retained_kill(environment_id, argv)
        }
        fn run_retained_cleanup<'a>(
            &'a self,
            environment_id: &'a str,
            argv: &'a [String],
        ) -> PortFuture<'a, ()> {
            self.0.run_retained_cleanup(environment_id, argv)
        }
    }
    let finalize_after = |after: HostedSwarmRun| {
        let port = Arc::new(ExpiringPort(
            HostedRunPort::hosting(Some(running_swarm())),
            after,
        ));
        let registry = EnvironmentRegistry::new();
        let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
        let use_case = use_case(&registry, port.clone());
        block_on(use_case.finalize_member(
            &env_ref,
            "coordinator-uuid",
            None,
            MemberFinalizeMode::Exit,
        ));
        (port, registry, env_ref)
    };

    let expired = with_status(RunStatus::Paused, Some(RunStatus::BudgetExhausted));
    let (port, registry, env_ref) = finalize_after(expired);
    assert!(port.0.kills.lock().unwrap().is_empty());
    let reason = reason(&registry, &env_ref);
    assert!(
        reason.starts_with("run ended: budget-exhausted"),
        "{reason}"
    );

    let (port, registry, env_ref) = finalize_after(with_status(RunStatus::Blocked, None));
    assert_eq!(port.0.kills.lock().unwrap().len(), 1, "closed in between");
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
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
    // agent_cmd kill of that ONE member: the run has not ended, so the box
    // survives for a resume, and a deliberate kill is not a lost harness.
    // (A closed run and the owner's fleet teardown are pinned above, #2070.)
    for hosted in [
        running_swarm(),
        with_status(RunStatus::Paused, Some(RunStatus::Succeeded)),
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
    let use_case = use_case(&registry, port.clone());
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
    let use_case = use_case(&registry, port.clone());

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
    let use_case = use_case(&registry, port.clone());
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

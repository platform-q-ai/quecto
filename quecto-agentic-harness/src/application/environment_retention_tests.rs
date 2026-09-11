//! #1924: a swarm's environment is retained after every end of its run —
//! orderly or by loss of its coordinator — instead of being killed with the
//! final member; only an explicit kill closes it.

use std::sync::{Arc, Mutex};

use super::environment_finalization_tests::{ScriptCall, block_on, committed_env};
use crate::domain::environment_finalization::{
    EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, HostedSwarmRun,
    MemberFinalizeMode, SwarmRunObservation, retains_environment,
};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::subagent_launch::LaunchFuture;
use crate::domain::swarm::RunStatus;

/// A port whose environment hosts the configured swarm run (or none) and
/// records every lost-coordinator record it is asked to make.
struct HostedRunPort {
    observed: SwarmRunObservation,
    loss_result: Result<(), String>,
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
            loss_result: Ok(()),
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
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.losses.lock().unwrap().push(hosted.coordinator.clone());
            self.loss_result.clone()
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

#[test]
fn retention_policy_holds_only_for_an_exit_that_empties_a_resumable_run() {
    let running = running_swarm();
    let paused = HostedSwarmRun {
        status: RunStatus::Paused,
        ..running_swarm()
    };
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    let closed = HostedSwarmRun {
        status: RunStatus::Failed,
        ..running_swarm()
    };
    let run = SwarmRunObservation::Run;
    let unreadable = SwarmRunObservation::Unreadable("locked".to_string());
    assert!(retains_environment(
        MemberFinalizeMode::Exit,
        &run(running.clone())
    ));
    assert!(retains_environment(MemberFinalizeMode::Exit, &run(paused)));
    assert!(!retains_environment(
        MemberFinalizeMode::Exit,
        &run(placeholder)
    ));
    assert!(!retains_environment(MemberFinalizeMode::Exit, &run(closed)));
    assert!(!retains_environment(
        MemberFinalizeMode::Exit,
        &SwarmRunObservation::NoStore
    ));
    assert!(
        retains_environment(MemberFinalizeMode::Exit, &unreadable),
        "a store that exists but cannot be read is never proof of no live run"
    );
    for mode in [
        MemberFinalizeMode::ParentKill,
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        assert!(
            !retains_environment(mode, &run(running.clone())),
            "{mode:?} is supervisor-initiated and keeps its teardown"
        );
        assert!(!retains_environment(mode, &unreadable));
    }
}

#[test]
fn coordinator_exit_with_running_swarm_withholds_kill_and_retains_record() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));

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
    let reason = record.metadata["retained"].as_str().unwrap();
    assert!(reason.contains("member-42"), "{reason}");
    assert!(reason.contains("kill_container"), "{reason}");
    // Only an explicit kill may now run the retained kill.
    assert!(registry.begin_kill(&env_ref).is_ok());
}

#[test]
fn coordinator_exit_still_retains_when_the_store_refuses_the_loss_record() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let mut port = HostedRunPort::hosting(Some(running_swarm()));
    port.loss_result = Err("store locked".to_string());
    let port = Arc::new(port);
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));

    assert!(port.kills.lock().unwrap().is_empty());
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    assert!(
        record.metadata["retained"]
            .as_str()
            .unwrap()
            .contains("store locked")
    );
}

#[test]
fn plain_member_exit_without_a_swarm_keeps_the_final_member_kill() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a"]);
    let port = Arc::new(HostedRunPort::hosting(None));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(port.kills.lock().unwrap().len(), 1);
    assert!(port.losses.lock().unwrap().is_empty());
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn exit_from_a_container_at_its_bootstrap_placeholder_keeps_the_final_member_kill() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a"]);
    let placeholder = HostedSwarmRun {
        status: RunStatus::Setup,
        deadline: 0.0,
        ..running_swarm()
    };
    let port = Arc::new(HostedRunPort::hosting(Some(placeholder)));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

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
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());
    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained
    );

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
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let port = Arc::new(HostedRunPort::hosting(Some(running_swarm())));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());
    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));
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
fn coordinator_exit_after_an_orderly_end_retains_without_quarantine() {
    // The user's decision: every swarm end keeps its container for
    // inspection. A run already paused holding an outcome is not a loss.
    for outcome in [
        RunStatus::Succeeded,
        RunStatus::Blocked,
        RunStatus::Failed,
        RunStatus::BudgetExhausted,
    ] {
        let registry = EnvironmentRegistry::new();
        let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
        let ended = HostedSwarmRun {
            status: RunStatus::Paused,
            outcome: Some(outcome),
            ..running_swarm()
        };
        assert!(ended.ended());
        let port = Arc::new(HostedRunPort::hosting(Some(ended)));
        let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

        block_on(use_case.finalize_member(
            &env_ref,
            "coordinator-uuid",
            None,
            MemberFinalizeMode::Exit,
        ));

        assert!(port.kills.lock().unwrap().is_empty(), "{outcome:?}");
        assert!(
            port.losses.lock().unwrap().is_empty(),
            "an orderly end never quarantines the coordinator ({outcome:?})"
        );
        let record = registry.get(&env_ref).unwrap();
        assert_eq!(record.status, EnvironmentStatus::Retained);
        let reason = record.metadata["retained"].as_str().unwrap();
        assert!(reason.starts_with("run ended: "), "{reason}");
        assert!(!reason.contains("lost"), "{reason}");
        assert!(reason.contains("kill_container"), "{reason}");
    }
}

#[test]
fn coordinator_exit_from_a_plain_pause_is_a_loss() {
    // A supervisor pause holds no outcome: losing the coordinator there is a
    // loss, so the run must be paused holding failed with the blocker.
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let paused = HostedSwarmRun {
        status: RunStatus::Paused,
        outcome: None,
        ..running_swarm()
    };
    assert!(!paused.ended());
    let port = Arc::new(HostedRunPort::hosting(Some(paused)));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));

    assert_eq!(
        port.losses.lock().unwrap().as_slice(),
        &["member-42".to_string()]
    );
    let reason = registry.get(&env_ref).unwrap().metadata["retained"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        reason.contains("lost its connection while the run was paused"),
        "{reason}"
    );
}

#[test]
fn coordinator_exit_with_an_unreadable_store_retains_without_a_loss_record() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["coordinator-uuid"]);
    let port = Arc::new(HostedRunPort::observing(SwarmRunObservation::Unreadable(
        "coordination store unavailable or contended".to_string(),
    )));
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(
        &env_ref,
        "coordinator-uuid",
        None,
        MemberFinalizeMode::Exit,
    ));

    assert!(port.kills.lock().unwrap().is_empty());
    assert!(
        port.losses.lock().unwrap().is_empty(),
        "no run was observed, so nothing is quarantined"
    );
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    assert!(
        record.metadata["retained"]
            .as_str()
            .unwrap()
            .contains("contended")
    );
}

use std::sync::{Arc, Mutex};

use crate::domain::environment_finalization::{
    EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, MemberFinalizeMode,
};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::subagent_launch::LaunchFuture;

#[derive(Debug, PartialEq, Eq)]
enum ScriptEvent {
    Inspect,
    Kill,
    Cleanup,
}

#[derive(Debug, PartialEq, Eq)]
struct ScriptCall {
    environment_id: String,
    argv: Vec<String>,
}

struct SpyFinalizationPort {
    inspects: Mutex<Vec<ScriptCall>>,
    kills: Mutex<Vec<ScriptCall>>,
    cleanups: Mutex<Vec<ScriptCall>>,
    events: Mutex<Vec<ScriptEvent>>,
    members_observed_at_inspect: Mutex<Vec<Vec<String>>>,
    observed_registry: Option<EnvironmentRegistry>,
    observed_env_ref: Option<String>,
    inspect_result: Mutex<Result<serde_json::Value, String>>,
    fail_kill: bool,
}

impl Default for SpyFinalizationPort {
    fn default() -> Self {
        Self {
            inspects: Mutex::new(Vec::new()),
            kills: Mutex::new(Vec::new()),
            cleanups: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            members_observed_at_inspect: Mutex::new(Vec::new()),
            observed_registry: None,
            observed_env_ref: None,
            inspect_result: Mutex::new(Ok(serde_json::json!({}))),
            fail_kill: false,
        }
    }
}

impl EnvironmentFinalizationPort for SpyFinalizationPort {
    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(ScriptEvent::Inspect);
            if let (Some(registry), Some(env_ref)) =
                (&self.observed_registry, &self.observed_env_ref)
            {
                let members = registry
                    .get(env_ref)
                    .map(|record| record.members)
                    .unwrap_or_default();
                self.members_observed_at_inspect
                    .lock()
                    .unwrap()
                    .push(members);
            }
            self.inspects.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            self.inspect_result.lock().unwrap().clone()
        })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.events.lock().unwrap().push(ScriptEvent::Kill);
            self.kills.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            if self.fail_kill {
                Err("retained kill failed".to_string())
            } else {
                Ok(())
            }
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()> {
        Box::pin(async move {
            self.events.lock().unwrap().push(ScriptEvent::Cleanup);
            self.cleanups.lock().unwrap().push(ScriptCall {
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
        })
    }
}

fn committed_env_with_kill(
    registry: &EnvironmentRegistry,
    members: Vec<&str>,
    retained_kill_argv: Vec<String>,
) -> String {
    committed_env_with_scripts(registry, members, retained_kill_argv, vec![])
}

fn committed_env_with_scripts(
    registry: &EnvironmentRegistry,
    members: Vec<&str>,
    retained_kill_argv: Vec<String>,
    retained_inspect_argv: Vec<String>,
) -> String {
    let env_ref = registry.mint_ref();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: std::path::PathBuf::from(format!("/ws/{env_ref}")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv,
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv,
        members: members.into_iter().map(str::to_string).collect(),
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    env_ref
}

fn committed_env(registry: &EnvironmentRegistry, members: Vec<&str>) -> String {
    committed_env_with_kill(registry, members, vec!["kill.sh".to_string()])
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}

#[test]
fn final_member_kill_is_orchestrated_by_application_use_case_without_cleanup() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a"]);
    let expected_environment_id = registry.get(&env_ref).unwrap().environment_id;
    let port = Arc::new(SpyFinalizationPort::default());
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(
        port.kills.lock().unwrap().as_slice(),
        &[ScriptCall {
            environment_id: expected_environment_id,
            argv: vec!["kill.sh".to_string()],
        }]
    );
    assert!(port.cleanups.lock().unwrap().is_empty());
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(record.members.is_empty());
}

#[test]
fn non_final_member_removal_does_not_finalize_environment() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a", "agent-b"]);
    let port = Arc::new(SpyFinalizationPort::default());
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert!(port.kills.lock().unwrap().is_empty());
    assert!(port.cleanups.lock().unwrap().is_empty());
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Running);
    assert_eq!(record.members, vec!["agent-b".to_string()]);
}

#[test]
fn failed_final_member_kill_persists_cleanup_failed_state() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env(&registry, vec!["agent-a"]);
    let port = Arc::new(SpyFinalizationPort {
        fail_kill: true,
        ..Default::default()
    });
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(port.kills.lock().unwrap().len(), 1);
    assert!(port.cleanups.lock().unwrap().is_empty());
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(record.last_error.as_deref(), Some("retained kill failed"));
}

#[test]
fn final_member_without_retained_kill_uses_retained_cleanup_fallback() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env_with_kill(&registry, vec!["agent-a"], vec![]);
    let expected_environment_id = registry.get(&env_ref).unwrap().environment_id;
    let port = Arc::new(SpyFinalizationPort::default());
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert!(port.kills.lock().unwrap().is_empty());
    assert_eq!(
        port.cleanups.lock().unwrap().as_slice(),
        &[ScriptCall {
            environment_id: expected_environment_id,
            argv: vec!["cleanup.sh".to_string()],
        }]
    );
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn exit_finalization_runs_retained_inspect_before_removal_and_merges_metadata() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env_with_scripts(
        &registry,
        vec!["agent-a"],
        vec!["kill.sh".to_string()],
        vec!["inspect.sh".to_string(), "--json".to_string()],
    );
    let expected_environment_id = registry.get(&env_ref).unwrap().environment_id;
    let port = Arc::new(SpyFinalizationPort {
        members_observed_at_inspect: Mutex::new(Vec::new()),
        observed_registry: Some(registry.clone()),
        observed_env_ref: Some(env_ref.clone()),
        inspect_result: Mutex::new(Ok(serde_json::json!({"postmortem": "captured"}))),
        ..Default::default()
    });
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(
        port.inspects.lock().unwrap().as_slice(),
        &[ScriptCall {
            environment_id: expected_environment_id,
            argv: vec!["inspect.sh".to_string(), "--json".to_string()],
        }]
    );
    assert_eq!(
        port.events.lock().unwrap().as_slice(),
        &[ScriptEvent::Inspect, ScriptEvent::Kill]
    );
    assert_eq!(
        port.members_observed_at_inspect.lock().unwrap().as_slice(),
        &[vec!["agent-a".to_string()]],
        "inspect must run before remove_member deletes the final member"
    );
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert_eq!(record.metadata["postmortem"], "captured");
    assert!(record.last_error.is_none());
}

#[test]
fn exit_finalization_persists_inspect_failure_even_after_successful_kill() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env_with_scripts(
        &registry,
        vec!["agent-a"],
        vec!["kill.sh".to_string()],
        vec!["inspect.sh".to_string()],
    );
    let port = Arc::new(SpyFinalizationPort {
        inspect_result: Mutex::new(Err("inspect failed".to_string())),
        ..Default::default()
    });
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(
        port.kills.lock().unwrap().as_slice(),
        &[ScriptCall {
            environment_id: registry.get(&env_ref).unwrap().environment_id,
            argv: vec!["kill.sh".to_string()],
        }]
    );
    assert_eq!(
        port.events.lock().unwrap().as_slice(),
        &[ScriptEvent::Inspect, ScriptEvent::Kill]
    );
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert_eq!(record.last_error.as_deref(), Some("inspect failed"));
    assert_eq!(record.retained_inspect_argv, vec!["inspect.sh".to_string()]);
}

#[test]
fn non_exit_finalization_does_not_run_retained_inspect() {
    for mode in [
        MemberFinalizeMode::ParentKill,
        MemberFinalizeMode::LaunchRollback,
        MemberFinalizeMode::LaunchRollbackOwned,
    ] {
        let registry = EnvironmentRegistry::new();
        let env_ref = committed_env_with_scripts(
            &registry,
            vec!["agent-a"],
            vec!["kill.sh".to_string()],
            vec!["inspect.sh".to_string()],
        );
        let port = Arc::new(SpyFinalizationPort::default());
        let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

        block_on(use_case.finalize_member(&env_ref, "agent-a", None, mode));

        assert!(
            port.inspects.lock().unwrap().is_empty(),
            "{mode:?} must not inspect; events: {:?}",
            port.events.lock().unwrap()
        );
    }
}

#[test]
fn duplicate_exit_finalization_inspects_dead_member_only_once() {
    let registry = EnvironmentRegistry::new();
    let env_ref = committed_env_with_scripts(
        &registry,
        vec!["agent-a", "agent-b"],
        vec!["kill.sh".to_string()],
        vec!["inspect.sh".to_string()],
    );
    let port = Arc::new(SpyFinalizationPort::default());
    let use_case = EnvironmentFinalizationUseCase::new(registry.clone(), port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));
    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(port.inspects.lock().unwrap().len(), 1);
    assert!(port.kills.lock().unwrap().is_empty());
    assert_eq!(
        registry.get(&env_ref).unwrap().members,
        vec!["agent-b".to_string()]
    );
}

// ── #1924: a lost coordinator retains a resumable swarm's environment ────

use crate::domain::environment_finalization::{
    HostedSwarmRun, SwarmRunObservation, retains_environment,
};
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
fn a_join_revives_a_retained_environment_and_a_kill_claim_closes_it() {
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

    registry.add_member(&env_ref, "relaunched-uuid").unwrap();
    let record = registry.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Running);
    assert_eq!(record.members, vec!["relaunched-uuid".to_string()]);

    let claim = registry.begin_kill(&env_ref).unwrap();
    registry.complete_kill(claim);
    assert_eq!(
        registry.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
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

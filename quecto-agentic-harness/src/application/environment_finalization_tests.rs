use std::sync::{Arc, Mutex};

use crate::domain::environment_finalization::{
    EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, MemberFinalizeMode,
};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::subagent_launch::LaunchFuture;

#[derive(Debug, PartialEq, Eq)]
struct ScriptCall {
    environment_id: String,
    argv: Vec<String>,
}

#[derive(Default)]
struct SpyFinalizationPort {
    kills: Mutex<Vec<ScriptCall>>,
    cleanups: Mutex<Vec<ScriptCall>>,
    fail_kill: bool,
}

impl EnvironmentFinalizationPort for SpyFinalizationPort {
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
        retained_inspect_argv: vec![],
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

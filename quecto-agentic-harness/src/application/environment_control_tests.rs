//! Slice 2 (#1369): application-level environment control use case.
//!
//! `get_containers`, `kill_container`, and ref/name resolution are use cases;
//! UDS/tool handlers may only decode, delegate here, and encode.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::application::environment_control::{EnvironmentControlUseCase, EnvironmentKillPort};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
};
use crate::domain::subagent_launch::LaunchFuture;

#[derive(Default)]
struct SpyKillPort {
    calls: AtomicUsize,
    fail_first: bool,
}

impl EnvironmentKillPort for SpyKillPort {
    fn kill_environment<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                record.retained_kill_argv,
                vec!["kill.sh".to_string()],
                "kill must use the environment's retained kill argv"
            );
            if self.fail_first && n == 0 {
                Err("kill.sh exited 1".to_string())
            } else {
                Ok(())
            }
        })
    }
}

fn committed_env(reg: &EnvironmentRegistry) -> String {
    let env_ref = reg.mint_ref();
    reg.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: std::path::PathBuf::from(format!("/ws/{env_ref}")),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string()],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    env_ref
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn get_containers_lists_from_the_authoritative_registry() {
    let reg = EnvironmentRegistry::new();
    let running = committed_env(&reg);
    let stopped = committed_env(&reg);
    let claim = reg.begin_kill(&stopped).unwrap();
    reg.complete_kill(claim);
    let uc = EnvironmentControlUseCase::new(reg.clone(), Arc::new(SpyKillPort::default()));
    let listing = uc.get_containers();
    let statuses: Vec<(String, EnvironmentStatus)> = listing
        .iter()
        .map(|r| (r.environment_ref.clone(), r.status.clone()))
        .collect();
    assert!(statuses.contains(&(running.clone(), EnvironmentStatus::Running)));
    assert!(statuses.contains(&(stopped.clone(), EnvironmentStatus::Stopped)));
}

#[test]
fn kill_container_calls_the_retained_kill_exactly_once_and_commits_stopped() {
    let reg = EnvironmentRegistry::new();
    let env_ref = committed_env(&reg);
    let port = Arc::new(SpyKillPort::default());
    let uc = EnvironmentControlUseCase::new(reg.clone(), port.clone());
    block_on(uc.kill_container(&EnvironmentTarget::Ref(env_ref.clone()))).unwrap();
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn kill_container_unknown_ref_fails_without_invoking_kill() {
    let reg = EnvironmentRegistry::new();
    committed_env(&reg);
    let port = Arc::new(SpyKillPort::default());
    let uc = EnvironmentControlUseCase::new(reg, port.clone());
    let err = block_on(uc.kill_container(&EnvironmentTarget::Ref("C9".to_string()))).unwrap_err();
    assert!(err.to_string().contains("C9"), "{err}");
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn kill_failure_persists_retryable_state_and_retry_succeeds() {
    let reg = EnvironmentRegistry::new();
    let env_ref = committed_env(&reg);
    let port = Arc::new(SpyKillPort {
        calls: AtomicUsize::new(0),
        fail_first: true,
    });
    let uc = EnvironmentControlUseCase::new(reg.clone(), port.clone());
    let err = block_on(uc.kill_container(&EnvironmentTarget::Ref(env_ref.clone()))).unwrap_err();
    assert!(err.to_string().contains("kill.sh exited 1"), "{err}");
    let rec = reg.get(&env_ref).unwrap();
    assert_eq!(rec.status, EnvironmentStatus::CleanupFailed);
    assert_eq!(rec.last_error.as_deref(), Some("kill.sh exited 1"));
    // Retry succeeds and only then commits stopped.
    block_on(uc.kill_container(&EnvironmentTarget::Ref(env_ref.clone()))).unwrap();
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn concurrent_kill_container_calls_cannot_double_kill() {
    let reg = EnvironmentRegistry::new();
    let env_ref = committed_env(&reg);
    let port = Arc::new(SpyKillPort::default());
    let uc = Arc::new(EnvironmentControlUseCase::new(reg.clone(), port.clone()));
    let (a, b) = block_on(async {
        let ua = uc.clone();
        let ub = uc.clone();
        let ra = EnvironmentTarget::Ref(env_ref.clone());
        let rb = EnvironmentTarget::Ref(env_ref.clone());
        tokio::join!(ua.kill_container(&ra), ub.kill_container(&rb))
    });
    assert_eq!(port.calls.load(Ordering::SeqCst), 1, "exactly one kill");
    assert!(
        a.is_ok() || b.is_ok(),
        "at least one caller observes success"
    );
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[test]
fn resolve_target_supports_names_for_control_operations() {
    let reg = EnvironmentRegistry::new();
    let env_ref = reg.mint_ref();
    reg.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "runtime-x".to_string(),
        environment_uuid: "uuid-x".to_string(),
        name: Some("review-env".to_string()),
        workspace_path: std::path::PathBuf::from("/ws/x"),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string()],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    let port = Arc::new(SpyKillPort::default());
    let uc = EnvironmentControlUseCase::new(reg.clone(), port.clone());
    block_on(uc.kill_container(&EnvironmentTarget::Name("review-env".to_string()))).unwrap();
    assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        reg.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

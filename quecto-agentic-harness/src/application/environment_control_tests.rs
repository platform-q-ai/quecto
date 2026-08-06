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
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
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

/// Kill port that parks inside the kill until released, so a second
/// `kill_container` genuinely overlaps the first one's in-flight kill.
struct GatedKillPort {
    calls: AtomicUsize,
    gate: Arc<tokio::sync::Notify>,
}

impl EnvironmentKillPort for GatedKillPort {
    fn kill_environment<'a>(
        &'a self,
        _record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.gate.notified().await;
            Ok(())
        })
    }
}

#[test]
fn concurrent_kill_container_calls_cannot_double_kill() {
    let reg = EnvironmentRegistry::new();
    let env_ref = committed_env(&reg);
    let gate = Arc::new(tokio::sync::Notify::new());
    let port = Arc::new(GatedKillPort {
        calls: AtomicUsize::new(0),
        gate: gate.clone(),
    });
    let uc = Arc::new(EnvironmentControlUseCase::new(reg.clone(), port.clone()));
    let (a, b) = block_on(async {
        let ua = uc.clone();
        let ub = uc.clone();
        let ra = EnvironmentTarget::Ref(env_ref.clone());
        let rb = EnvironmentTarget::Ref(env_ref.clone());
        tokio::join!(ua.kill_container(&ra), async {
            // On this current-thread runtime the first caller has already
            // claimed the kill and parked inside the port when this runs, so
            // the second call races an IN-FLIGHT kill, not a finished one.
            let second = ub.kill_container(&rb).await;
            gate.notify_one();
            second
        })
    });
    assert_eq!(port.calls.load(Ordering::SeqCst), 1, "exactly one kill");
    assert!(a.is_ok(), "the claim holder completes its kill");
    let err = b.unwrap_err();
    assert!(
        err.contains("stale"),
        "the overlapping caller is refused while the claim is outstanding: {err}"
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
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
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

#[test]
fn kill_container_without_retained_kill_argv_fails_before_any_claim() {
    let reg = EnvironmentRegistry::new();
    let env_ref = reg.mint_ref();
    reg.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: format!("uuid-{env_ref}"),
        name: None,
        workspace_path: std::path::PathBuf::from("/ws/kill-less"),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv: vec![],
        members: vec!["agent-a".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    let port = Arc::new(SpyKillPort::default());
    let uc = EnvironmentControlUseCase::new(reg.clone(), port.clone());

    let err = block_on(uc.kill_container(&EnvironmentTarget::Ref(env_ref.clone()))).unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);

    // No state transition: still Running with its member, so joins keep
    // working and the final-member exit cleanup fallback stays reachable.
    let record = reg.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Running);
    assert_eq!(record.members, vec!["agent-a".to_string()]);
    assert!(
        reg.resolve(&EnvironmentTarget::Ref(env_ref.clone()))
            .is_ok()
    );
    // The final-member removal still mints the cleanup claim.
    let claim = reg.remove_member(&env_ref, "agent-a").unwrap();
    assert!(claim.is_some());
}

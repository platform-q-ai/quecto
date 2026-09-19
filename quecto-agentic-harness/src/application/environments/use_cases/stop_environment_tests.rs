use std::sync::{Arc, Mutex};

use crate::application::environments::ports::{
    EnvironmentMemberShutdown, EnvironmentProcessCommands, MemberShutdownReport, PortFuture,
};
use crate::application::environments::use_cases::{StopEnvironment, StopEnvironmentError};
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
};

#[derive(Default)]
struct StopSpy(Mutex<Vec<String>>);

impl EnvironmentProcessCommands for StopSpy {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { unreachable!() })
    }

    fn run_retained_stop<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async move {
            assert_eq!(environment_id, "env-1");
            assert_eq!(argv, ["kill.sh", "--op", "kill"]);
            self.0.lock().unwrap().push("stop".to_string());
            Ok(())
        })
    }

    fn run_retained_kill<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>> {
        Box::pin(async { unreachable!() })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> PortFuture<'a, ()> {
        Box::pin(async { unreachable!() })
    }
}

struct NoMembers;
impl EnvironmentMemberShutdown for NoMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        assert!(members.is_empty());
        Box::pin(async { MemberShutdownReport::default() })
    }
}

fn registry(status: EnvironmentStatus) -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    registry.commit(EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-1".into(),
        environment_uuid: "uuid-1".into(),
        name: Some("delivery".into()),
        workspace_path: "/state/env-1/workspace".into(),
        repository: "repo".into(),
        script_name: "standard".into(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into(), "--op".into(), "kill".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status,
        metadata: serde_json::json!({"checkout":"/state/env-1/workspace/repo"}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "session".into(),
        created_at: None,
    });
    registry
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn retained_environment_stops_runtime_and_becomes_non_joinable_preserved_data() {
    let registry = registry(EnvironmentStatus::Retained);
    let commands = Arc::new(StopSpy::default());
    let use_case = StopEnvironment::new(registry.clone(), Arc::new(NoMembers), commands.clone());

    let result = block_on(use_case.stop_container(&EnvironmentTarget::Ref("C1".into()))).unwrap();

    assert_eq!(result.record.environment_ref, "C1");
    assert_eq!(commands.0.lock().unwrap().as_slice(), ["stop"]);
    let preserved = registry.get("C1").unwrap();
    assert_eq!(preserved.status, EnvironmentStatus::Preserved);
    assert_eq!(
        preserved.workspace_path,
        std::path::PathBuf::from("/state/env-1/workspace")
    );
    assert!(
        registry
            .resolve_joinable(&EnvironmentTarget::Ref("C1".into()))
            .is_err()
    );
    assert!(
        registry.begin_kill("C1").is_ok(),
        "explicit discard remains available"
    );
}

#[test]
fn only_affirmatively_retained_environment_is_eligible() {
    for status in [
        EnvironmentStatus::Running,
        EnvironmentStatus::CleanupFailed,
        EnvironmentStatus::Stopped,
        EnvironmentStatus::Preserved,
    ] {
        let registry = registry(status.clone());
        let commands = Arc::new(StopSpy::default());
        let use_case = StopEnvironment::new(registry, Arc::new(NoMembers), commands.clone());
        let error = block_on(use_case.stop_container(&EnvironmentTarget::Ref("C1".into())))
            .expect_err("non-retained state must be refused");
        assert!(matches!(error, StopEnvironmentError::Refused(_)));
        assert!(commands.0.lock().unwrap().is_empty());
    }
}

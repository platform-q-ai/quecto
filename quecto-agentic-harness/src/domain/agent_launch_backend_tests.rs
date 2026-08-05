use super::agent_launch_backend::{
    AgentLaunchBackend, LocalProcessLaunchBackend, ScriptManagedContainerLaunchBackend,
};
use crate::domain::container_runtime::{ExistingContainerRef, SpawnContainerRequest};

#[test]
fn local_backend_accepts_only_local_requests() {
    let backend = LocalProcessLaunchBackend;
    assert_eq!(backend.backend_name(), "local");
    assert!(backend.can_launch(&SpawnContainerRequest::Local));
    assert!(!backend.can_launch(&SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    }));
    assert!(!backend.can_launch(&SpawnContainerRequest::Existing {
        reference: ExistingContainerRef::Ref("C1".into()),
    }));
    assert_eq!(backend.build_exec_command(), None);
}

#[test]
fn script_backend_exposes_launch_seam_for_container_exec() {
    let backend = ScriptManagedContainerLaunchBackend::default();
    assert_eq!(backend.backend_name(), "container-script");
    assert!(backend.can_launch(&SpawnContainerRequest::New {
        repo: None,
        container_script: None,
    }));
    assert_eq!(
        backend.build_exec_command(),
        Some("script-managed-container")
    );
}

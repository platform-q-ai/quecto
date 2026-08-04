use super::agent_launch_backend::{AgentLaunchBackend, LocalProcessLaunchBackend};
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
}

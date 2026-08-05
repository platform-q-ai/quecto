use quecto::domain::agent_launch_backend::{AgentLaunchBackend, LocalProcessLaunchBackend};

#[test]
fn local_backend_advertises_local_runtime() {
    let backend = LocalProcessLaunchBackend;
    assert_eq!(backend.backend_name(), "local");
    assert!(backend.can_launch(&quecto::domain::container_runtime::SpawnContainerRequest::Local));
    assert!(!backend.can_launch(
        &quecto::domain::container_runtime::SpawnContainerRequest::New {
            repo: None,
            container_script: None
        }
    ));
}

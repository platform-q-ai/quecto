use quecto::domain::container_runtime::{AgentLaunchBackend, LocalProcessLaunchBackend};

#[test]
fn local_backend_advertises_local_runtime() {
    let backend = LocalProcessLaunchBackend;
    assert_eq!(backend.backend_name(), "local");
}

use crate::domain::container_runtime::SpawnContainerRequest;

pub trait AgentLaunchBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool;
}

#[derive(Debug, Default)]
pub struct LocalProcessLaunchBackend;

impl AgentLaunchBackend for LocalProcessLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "local"
    }
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool {
        matches!(request, SpawnContainerRequest::Local)
    }
}

#[derive(Debug, Default)]
pub struct ScriptManagedContainerLaunchBackend;

impl AgentLaunchBackend for ScriptManagedContainerLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "container-script"
    }

    fn can_launch(&self, request: &SpawnContainerRequest) -> bool {
        matches!(
            request,
            SpawnContainerRequest::New { .. } | SpawnContainerRequest::Existing { .. }
        )
    }
}

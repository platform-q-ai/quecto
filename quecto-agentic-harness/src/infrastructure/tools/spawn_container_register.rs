use crate::domain::agent_launch_backend::ContainerLaunchOutcome;
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::infrastructure::tools::container_registry::{
    ContainerEntry, ContainerRegistry, ContainerStatus, register_container,
};
use crate::infrastructure::tools::spawn_container_existing::ExistingContainerJoin;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;

pub(crate) fn registered_container_for_launch(
    registry: &ContainerRegistry,
    launch: &ContainerLaunchOutcome,
    agent_uuid: &AgentUuid,
    existing_join: Option<&ExistingContainerJoin>,
) -> Result<ContainerEntry, DomainError> {
    if let Some(join) = existing_join {
        let state = registry.lock().unwrap_or_else(|e| e.into_inner());
        return state.entries.get(&join.uuid).cloned().ok_or_else(|| {
            DomainError::Tool(format!(
                "container '{}' disappeared during launch",
                join.uuid
            ))
        });
    }
    Ok(register_container(
        registry,
        ContainerEntry {
            container_uuid: launch
                .container_id
                .clone()
                .unwrap_or_else(|| launch.environment_id.clone()),
            container_ref: launch.container_ref.clone().unwrap_or_default(),
            container_name: launch.container_name.clone(),
            environment_id: launch.environment_id.clone(),
            repo_url: launch.repository.clone(),
            workspace_path: launch.workspace_path.display().to_string(),
            status: ContainerStatus::Running,
            agents: vec![agent_uuid.clone()],
            script_name: launch.script_name.clone(),
            exec_command: launch.exec_command.clone(),
            inspect_command: launch.inspect_command.clone(),
            kill_command: launch.kill_command.clone(),
            socket_path: launch.socket_path.as_ref().map(|p| p.display().to_string()),
            socket_proxy: launch.socket_proxy.clone(),
            metadata: launch.metadata.clone(),
        },
    ))
}

pub(crate) fn apply_launch_to_entry(
    entry: &mut SubagentEntry,
    registered_container: &ContainerEntry,
    launch: &ContainerLaunchOutcome,
    endpoint: &crate::domain::agent_launch_backend::ParentEndpoint,
) {
    entry.runtime_backend = "container".to_string();
    entry.container_uuid = Some(registered_container.container_uuid.clone());
    entry.container_ref = Some(registered_container.container_ref.clone());
    entry.container_name = launch.container_name.clone();
    entry.repo_url = launch.repository.clone();
    entry.environment_id = Some(launch.environment_id.clone());
    entry.environment_health = Some(
        launch
            .status
            .clone()
            .unwrap_or_else(|| "running".to_string()),
    );
    entry.socket_mode = Some(endpoint.mode().to_string());
    entry.parent_endpoint = Some(endpoint.clone());
    entry.workspace_path = Some(launch.workspace_path.display().to_string());
    entry.container_script_name = Some(launch.script_name.clone());
    entry.container_kill_command = Some(launch.kill_command.clone());
    entry.container_inspect_command = Some(launch.inspect_command.clone());
}

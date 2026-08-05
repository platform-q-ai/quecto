use crate::domain::agent_launch_backend::PreparedAgentLaunch;
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;

pub(super) async fn await_prepared_launch_owner(
    mut prepared: PreparedAgentLaunch,
    base_dir: &std::path::Path,
    container_registry: &crate::infrastructure::tools::container_registry::ContainerRegistry,
    existing_join: Option<&super::spawn_container_existing::ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) -> Result<(PreparedAgentLaunch, Option<tokio::process::Child>, u32), DomainError> {
    let owned_child = if let Some(mut cmd) = prepared.command.take() {
        if !base_dir.as_os_str().is_empty() {
            cmd.env("QUECTO_BASE_DIR", base_dir);
        }
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                super::spawn::rollback_existing_join(container_registry, existing_join, agent_uuid);
                return Err(DomainError::Tool(format!("failed to spawn subagent: {e}")));
            }
        };
        if let Err(e) =
            super::spawn_wait::wait_for_endpoint_or_child_exit(&prepared.endpoint, &mut child).await
        {
            let _ = child.kill().await;
            let _ = child.wait().await;
            super::spawn::rollback_existing_join(container_registry, existing_join, agent_uuid);
            return Err(e);
        }
        Some(child)
    } else {
        wait_for_script_owned_endpoint(&prepared, container_registry, existing_join, agent_uuid)
            .await?;
        None
    };
    let pid = owned_child
        .as_ref()
        .and_then(tokio::process::Child::id)
        .unwrap_or(0);
    Ok((prepared, owned_child, pid))
}

async fn wait_for_script_owned_endpoint(
    prepared: &PreparedAgentLaunch,
    container_registry: &crate::infrastructure::tools::container_registry::ContainerRegistry,
    existing_join: Option<&super::spawn_container_existing::ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) -> Result<(), DomainError> {
    if let Err(e) =
        super::parent_endpoint::wait_ready(&prepared.endpoint, std::time::Duration::from_secs(10))
            .await
    {
        let cleanup =
            rollback_script_owned_launch(prepared, container_registry, existing_join, agent_uuid)
                .await;
        return match cleanup {
            Ok(()) => Err(e),
            Err(cleanup_err) => Err(DomainError::Tool(format!(
                "{e}; rollback cleanup failed: {cleanup_err}"
            ))),
        };
    }
    Ok(())
}

async fn rollback_script_owned_launch(
    prepared: &PreparedAgentLaunch,
    container_registry: &crate::infrastructure::tools::container_registry::ContainerRegistry,
    existing_join: Option<&super::spawn_container_existing::ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) -> Result<(), String> {
    super::spawn::rollback_existing_join(container_registry, existing_join, agent_uuid);
    if existing_join.is_some() {
        return Ok(());
    }
    let Some(launch) = prepared.container.as_ref() else {
        return Ok(());
    };
    let mut entry = super::subagent_registry::SubagentEntry::new(
        prepared
            .endpoint
            .socket_path()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default(),
        0,
    );
    entry.agent_uuid = agent_uuid.clone();
    entry.runtime_backend = "container".into();
    entry.container_uuid = launch
        .container_id
        .clone()
        .or_else(|| Some(launch.environment_id.clone()));
    entry.container_ref = launch.container_ref.clone();
    entry.container_name = launch.container_name.clone();
    entry.repo_url = launch.repository.clone();
    entry.environment_id = Some(launch.environment_id.clone());
    entry.workspace_path = Some(launch.workspace_path.display().to_string());
    entry.container_script_name = Some(launch.script_name.clone());
    entry.container_kill_command = Some(launch.kill_command.clone());
    entry.container_inspect_command = Some(launch.inspect_command.clone());
    super::container_script_cleanup::invoke_container_kill_script(&entry)
}

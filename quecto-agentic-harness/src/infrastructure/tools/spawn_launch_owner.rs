use crate::domain::agent_launch_backend::{ParentEndpoint, PreparedAgentLaunch};
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
        wait_for_script_owned_endpoint(
            &prepared.endpoint,
            container_registry,
            existing_join,
            agent_uuid,
        )
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
    endpoint: &ParentEndpoint,
    container_registry: &crate::infrastructure::tools::container_registry::ContainerRegistry,
    existing_join: Option<&super::spawn_container_existing::ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) -> Result<(), DomainError> {
    if let Err(e) =
        super::parent_endpoint::wait_ready(endpoint, std::time::Duration::from_secs(10)).await
    {
        super::spawn::rollback_existing_join(container_registry, existing_join, agent_uuid);
        return Err(e);
    }
    Ok(())
}

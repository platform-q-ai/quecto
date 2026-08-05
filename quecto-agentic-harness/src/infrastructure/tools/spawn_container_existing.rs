use crate::domain::container_runtime::{ExistingContainerRef, SpawnContainerRequest};
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::infrastructure::tools::container_registry::{
    ContainerRegistry, add_agent_to_live_container, remove_agent_from_container, resolve_live_ref,
};

#[derive(Debug, Clone)]
pub(crate) struct ExistingContainerJoin {
    pub uuid: String,
    pub entry: crate::infrastructure::tools::container_registry::ContainerEntry,
}

impl ExistingContainerJoin {
    pub(crate) fn environment_id(&self) -> &str {
        &self.entry.environment_id
    }
}

pub(crate) fn prepare_existing_container_join(
    registry: &ContainerRegistry,
    request: &SpawnContainerRequest,
    agent_uuid: &AgentUuid,
) -> Result<Option<ExistingContainerJoin>, DomainError> {
    let Some(live_ref) = existing_ref(request) else {
        return Ok(None);
    };
    let uuid = resolve_live_ref(registry, live_ref).map_err(DomainError::Tool)?;
    add_agent_to_live_container(registry, &uuid, agent_uuid.clone()).map_err(DomainError::Tool)?;
    let entry = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .get(&uuid)
        .cloned()
        .ok_or_else(|| DomainError::Tool(format!("container '{uuid}' disappeared during join")))?;
    Ok(Some(ExistingContainerJoin { uuid, entry }))
}

pub(crate) fn rollback_existing_container_join(
    registry: &ContainerRegistry,
    join: Option<&ExistingContainerJoin>,
    agent_uuid: &AgentUuid,
) {
    if let Some(join) = join {
        let _ = remove_agent_from_container(registry, &join.uuid, agent_uuid);
    }
}

fn existing_ref(request: &SpawnContainerRequest) -> Option<&str> {
    match request {
        SpawnContainerRequest::Existing { reference } => match reference {
            ExistingContainerRef::Ref(r) | ExistingContainerRef::Name(r) => Some(r.as_str()),
        },
        _ => None,
    }
}

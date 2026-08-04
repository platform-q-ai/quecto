use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::ids::AgentUuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerStatus {
    Running,
    Stopped,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    pub container_uuid: String,
    pub container_ref: String,
    pub container_name: Option<String>,
    pub environment_id: String,
    pub repo_url: Option<String>,
    pub workspace_path: String,
    pub status: ContainerStatus,
    pub agents: Vec<AgentUuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Default)]
pub struct ContainerRegistryState {
    next_ref: u64,
    entries: HashMap<String, ContainerEntry>,
}

pub type ContainerRegistry = Arc<Mutex<ContainerRegistryState>>;

pub fn new_container_registry() -> ContainerRegistry {
    Arc::new(Mutex::new(ContainerRegistryState::default()))
}

pub fn register_container(
    registry: &ContainerRegistry,
    mut entry: ContainerEntry,
) -> ContainerEntry {
    let mut state = registry.lock().unwrap_or_else(|e| e.into_inner());
    state.next_ref += 1;
    entry.container_ref = format!("C{}", state.next_ref);
    state
        .entries
        .insert(entry.container_uuid.clone(), entry.clone());
    entry
}

pub fn resolve_live_ref(
    registry: &ContainerRegistry,
    container_ref: &str,
) -> Result<String, String> {
    let state = registry.lock().unwrap_or_else(|e| e.into_inner());
    let entry = state
        .entries
        .values()
        .find(|e| e.container_ref == container_ref)
        .ok_or_else(|| format!("unknown container ref '{container_ref}'"))?;
    if entry.status != ContainerStatus::Running {
        return Err(format!("container ref '{container_ref}' is not live"));
    }
    Ok(entry.container_uuid.clone())
}

pub fn list_containers(registry: &ContainerRegistry) -> Vec<ContainerEntry> {
    let mut entries: Vec<_> = registry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entries
        .values()
        .cloned()
        .collect();
    entries.sort_by_key(|e| {
        e.container_ref
            .trim_start_matches('C')
            .parse::<u64>()
            .unwrap_or(u64::MAX)
    });
    entries
}

use crate::domain::agent_launch_backend::ParentEndpoint;

use super::subagent_registry::{SubagentEntry, SubagentRegistry};

pub(crate) fn endpoint_or_record_proxy_failure(
    registry: &SubagentRegistry,
    agent_id: &str,
    socket_path: std::path::PathBuf,
) -> Option<ParentEndpoint> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    match entries.get(agent_id) {
        Some(entry) if entry.parent_endpoint.is_some() => entry.parent_endpoint.clone(),
        Some(entry) if entry.socket_mode.as_deref() == Some("proxy") => {
            drop(entries);
            super::container_script_cleanup::record_container_health_failure(
                registry,
                [agent_id.to_string()],
                "connection_failed",
                "proxy endpoint missing; refusing direct UDS fallback".into(),
            );
            None
        }
        _ => Some(ParentEndpoint::DirectUds(socket_path)),
    }
}

pub(crate) fn endpoint_or_proxy_error(
    entry: &SubagentEntry,
    socket_path: std::path::PathBuf,
    agent_id: &str,
) -> Result<ParentEndpoint, String> {
    if let Some(endpoint) = entry.parent_endpoint.clone() {
        return Ok(endpoint);
    }
    if entry.socket_mode.as_deref() == Some("proxy") {
        return Err(format!(
            "agent '{agent_id}' has proxy socket mode but no validated proxy endpoint"
        ));
    }
    Ok(ParentEndpoint::DirectUds(socket_path))
}

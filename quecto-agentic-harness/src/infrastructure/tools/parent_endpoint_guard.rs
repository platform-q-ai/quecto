use crate::domain::agent_launch_backend::ParentEndpoint;

use super::subagent_registry::SubagentEntry;

pub(crate) fn endpoint_or_proxy_error(
    entry: &SubagentEntry,
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
    Ok(ParentEndpoint::DirectUds(entry.socket_path.clone()))
}

pub(crate) fn lookup_endpoint_for_agent(
    registry: &super::subagent_registry::SubagentRegistry,
    agent_id: &str,
) -> Result<ParentEndpoint, String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    let registry_key = super::subagent_registry::resolve_registry_key(&entries, agent_id)
        .map_err(|_| format!("subagent '{agent_id}' not found in registry"))?;
    let entry = entries
        .get(&registry_key)
        .ok_or_else(|| format!("agent '{agent_id}' endpoint disappeared"))?;
    endpoint_or_proxy_error(entry, agent_id)
}

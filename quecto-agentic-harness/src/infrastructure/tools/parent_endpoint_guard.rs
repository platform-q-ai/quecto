use crate::domain::agent_launch_backend::ParentEndpoint;

use super::subagent_registry::SubagentEntry;

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

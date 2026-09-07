//! CLI composition of container identity and the shared lifecycle adapter.
use super::AgentFlags;
use crate::infrastructure::tools::{swarm_bridge, swarm_lifecycle};

pub(super) fn admit(flags: &AgentFlags, stderr: &mut String) -> bool {
    if let Some(context) = swarm_bridge::SwarmContext::discover() {
        if let Err(error) =
            swarm_lifecycle::join_current_process(&context, flags.socket_path.as_deref())
        {
            stderr.push_str(&format!("swarm admission rejected: {error}\n"));
            return false;
        }
    }
    true
}

pub(super) fn bind_socket(socket: &std::path::Path, stderr: &mut String) -> bool {
    swarm_bridge::set_process_socket(socket.to_path_buf());
    if let Some(context) = swarm_bridge::SwarmContext::discover() {
        if context.database().exists() {
            if let Err(error) =
                context.call("_socket", serde_json::json!([socket.to_string_lossy()]))
            {
                stderr.push_str(&format!("swarm endpoint registration failed: {error}\n"));
                return false;
            }
        }
    }
    true
}

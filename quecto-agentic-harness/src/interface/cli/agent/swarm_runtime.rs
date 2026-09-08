//! CLI composition of container identity and the shared lifecycle adapter.
use super::AgentFlags;
use crate::domain::swarm::CoordinationPort;
use crate::infrastructure::tools::{swarm_bridge, swarm_lifecycle};

pub(super) fn admit(flags: &AgentFlags, stderr: &mut String) -> bool {
    if let Some(context) = crate::interface::tool_runtime::swarm_context() {
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
    if let Some(context) = crate::interface::tool_runtime::swarm_context() {
        if context.database().exists() {
            if let Err(error) = context.register_endpoint(&socket.to_string_lossy()) {
                stderr.push_str(&format!("swarm endpoint registration failed: {error}\n"));
                return false;
            }
        }
    }
    true
}

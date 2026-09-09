//! CLI composition of container identity and the shared lifecycle adapter.
use super::AgentFlags;
use crate::domain::swarm::CoordinationPort;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use crate::infrastructure::tools::{swarm_bridge, swarm_lifecycle};

pub(super) fn admit(flags: &mut AgentFlags, stderr: &mut String) -> bool {
    admit_with(
        crate::interface::tool_runtime::swarm_context(),
        flags,
        stderr,
    )
}

/// Workflow eligibility follows swarm participation, not containerization
/// (#1715): an ordinary container keeps its workflow; a container whose run
/// has been created (or a worker launched into one) rejects it before joining.
pub(super) fn admit_with(
    context: Option<SwarmContext>,
    flags: &mut AgentFlags,
    stderr: &mut String,
) -> bool {
    let Some(context) = context else {
        return true;
    };
    let status = match context.run_status() {
        Ok(status) => status,
        Err(error) => {
            stderr.push_str(&format!("swarm admission rejected: {error}\n"));
            return false;
        }
    };
    if crate::domain::swarm::participates(status)
        && let Err(error) = disable_workflow(flags)
    {
        stderr.push_str(&format!("swarm admission rejected: {error}\n"));
        return false;
    }
    if let Err(error) =
        swarm_lifecycle::join_current_process(&context, flags.socket_path.as_deref())
    {
        stderr.push_str(&format!("swarm admission rejected: {error}\n"));
        return false;
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

fn disable_workflow(flags: &mut AgentFlags) -> Result<(), crate::domain::error::DomainError> {
    crate::domain::swarm::validate_workflow(
        true,
        flags.workflow || flags.workflow_guards || flags.workflow_spec_path.is_some(),
    )?;
    flags.workflow_disabled = true;
    Ok(())
}

#[cfg(test)]
#[path = "swarm_runtime_tests.rs"]
mod tests;

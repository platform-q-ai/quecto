//! CLI composition of container identity and the shared lifecycle adapter.
use super::AgentFlags;
use crate::application::swarm::ports::CoordinationPort;
use crate::domain::external_agent::backend::MemberBackend;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use crate::infrastructure::tools::{swarm_bridge, swarm_lifecycle};
use crate::interface::cli::CliContext;

/// Admit this process: a swarm member's board is composition's (#2278),
/// bound as the process's board before its context is discovered, and a
/// process launched under the container contract without one is refused.
pub(super) fn admit(ctx: &CliContext, flags: &mut AgentFlags, stderr: &mut String) -> bool {
    let contracted = SwarmContext::contracted_checkout().is_some();
    let context = match board_admission(board(ctx), contracted) {
        Ok(Some(board)) => {
            swarm_bridge::bind_process_board(board);
            crate::interface::tool_runtime::swarm_context()
        }
        Ok(None) => None,
        Err(refusal) => {
            stderr.push_str(refusal);
            return false;
        }
    };
    admit_with(context, swarm_lifecycle::is_creator(), flags, stderr)
}

/// The board a process is admitted with: composition's, when composed; a
/// process launched under the container contract (`contracted`) without
/// one is refused, since the interface never constructs the board.
pub(super) fn board_admission(
    board: Option<swarm_bridge::SwarmBoard>,
    contracted: bool,
) -> Result<Option<swarm_bridge::SwarmBoard>, &'static str> {
    match (board, contracted) {
        (Some(board), _) => Ok(Some(board)),
        (None, false) => Ok(None),
        (None, true) => Err(SWARM_BOARD_NOT_COMPOSED),
    }
}

/// The refusal for a swarm member whose run composed no board (#2278).
pub(super) const SWARM_BOARD_NOT_COMPOSED: &str = "agent: swarm board capability not composed\n";

/// The board composition's builders carried on `ctx` make (#2278): it
/// records in the session's event log when `ctx` also carries
/// composition's `board_op_log`.
pub(super) fn board(ctx: &CliContext) -> Option<swarm_bridge::SwarmBoard> {
    match (ctx.swarm_board, ctx.swarm_board_log) {
        (Some(build), Some(session_log)) => Some(swarm_bridge::SwarmBoard::with_session_log(
            build,
            session_log,
        )),
        (Some(build), None) => Some(swarm_bridge::SwarmBoard::new(build)),
        (None, _) => None,
    }
}

/// Workflow eligibility follows swarm participation, not containerization
/// (#1715). A workflow request is checked against a membership-free status
/// read BEFORE joining, so a refused process never leaves a member row behind
/// (a live row with a dead pid would fail the whole run at the next
/// reconcile). The join then records participation; a swarm member has its
/// workflow disabled, an ordinary container keeps it. The `--backend` is
/// decided the same way, before the join (#2287).
pub(super) fn admit_with(
    context: Option<SwarmContext>,
    creator: bool,
    flags: &mut AgentFlags,
    stderr: &mut String,
) -> bool {
    let Some(context) = context else {
        return admit_backend(flags.backend.unwrap_or_default(), false, creator, stderr);
    };
    let created = match context.run_created() {
        Ok(created) => created,
        Err(error) => {
            stderr.push_str(&format!("swarm admission rejected: {error}\n"));
            return false;
        }
    };
    if !admit_backend(flags.backend.unwrap_or_default(), created, creator, stderr) {
        return false;
    }
    if created && let Err(error) = disable_workflow(flags) {
        stderr.push_str(&format!("swarm admission rejected: {error}\n"));
        return false;
    }
    // The join records participation on the shared handle; the workflow flag
    // was already settled above (a created run implies participation).
    if let Err(error) = swarm_lifecycle::join_current_process(
        &context,
        flags.socket_path.as_deref(),
        flags.swarm_participation.clone(),
        creator,
    ) {
        stderr.push_str(&format!("swarm admission rejected: {error}\n"));
        return false;
    }
    // A created run implies participation; a claude-code member that did
    // not come to participate (the run ended meanwhile) is still refused.
    match (
        flags.backend.unwrap_or_default(),
        flags.swarm_participation.participating(),
    ) {
        (MemberBackend::Quecto, _) | (MemberBackend::ClaudeCode, true) => true,
        (MemberBackend::ClaudeCode, false) => {
            stderr.push_str(CLAUDE_CODE_NOT_A_SWARM_WORKER);
            false
        }
    }
}

/// The refusal for a claude-code member that is not a swarm worker
/// (#2287, O1).
pub(super) const CLAUDE_CODE_NOT_A_SWARM_WORKER: &str =
    "agent: --backend claude-code runs only as a swarm worker admitted to a created swarm run\n";

/// Whether this process may run `backend`, decided before it joins: a
/// claude-code member must be a swarm worker (O1), a process joining a
/// `created` run that is not the run's creator (its coordinator).
fn admit_backend(
    backend: MemberBackend,
    created: bool,
    creator: bool,
    stderr: &mut String,
) -> bool {
    let admitted = match (backend, created, creator) {
        (MemberBackend::Quecto, _, _) | (MemberBackend::ClaudeCode, true, false) => true,
        (MemberBackend::ClaudeCode, false, _) | (MemberBackend::ClaudeCode, true, true) => false,
    };
    if !admitted {
        stderr.push_str(CLAUDE_CODE_NOT_A_SWARM_WORKER);
    }
    admitted
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

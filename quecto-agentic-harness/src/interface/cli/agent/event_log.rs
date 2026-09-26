//! Which agents keep the audit log (#2150): a workflow session with a key,
//! as before; with `telemetry.event_log` on, every agent.
use std::path::Path;
use std::sync::Arc;

use crate::application::audit::ports::AuditSink;

/// The key an agent's audit log is filed under, or `None` when it keeps
/// none: its session's key, else (a session without one) its process.
pub(super) fn log_key(
    workflow: bool,
    ephemeral: bool,
    session_key: &str,
    event_log: bool,
) -> Option<String> {
    let workflow_log = workflow && !ephemeral && !session_key.is_empty();
    match (workflow_log, event_log, session_key.is_empty()) {
        (true, _, _) | (false, true, false) => Some(session_key.to_owned()),
        (false, true, true) => Some(format!("pid-{}", std::process::id())),
        (false, false, _) => None,
    }
}

/// Open the agent's audit log under `key`, naming a sub-agent's parent; a
/// failure to open is a warning, never a failed start.
pub(super) fn open(
    base_dir: &Path,
    key: &str,
    flags: &super::AgentFlags,
    stderr: &mut String,
) -> Option<Arc<dyn AuditSink>> {
    match crate::infrastructure::persistence::audit_log::AuditLog::open_sync(base_dir, key) {
        Ok(log) => Some(Arc::new(log.with_parent(flags.parent_id.clone())) as Arc<dyn AuditSink>),
        Err(e) => {
            stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
            None
        }
    }
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;

//! Which agents keep the audit log (#2150): a workflow session with a key,
//! as before; with `telemetry.event_log` on, every agent except one asked
//! to leave nothing behind (`-s -`, `--no-session`).
use std::path::Path;
use std::sync::Arc;

use crate::application::agent_loop::AgentLoopImpl;
use crate::application::audit::ports::AuditSink;

/// The key an agent's audit log is filed under, or `None` when it keeps
/// none: its session's key, else a key of its own (process and start time,
/// never another session's).
pub(super) fn log_key(
    workflow: bool,
    ephemeral: bool,
    session_key: &str,
    event_log: bool,
) -> Option<String> {
    let workflow_log = workflow && !ephemeral && !session_key.is_empty();
    match (
        workflow_log,
        event_log && !ephemeral,
        session_key.is_empty(),
    ) {
        (true, _, _) | (false, true, false) => Some(session_key.to_owned()),
        (false, true, true) => Some(unkeyed()),
        (false, false, _) => None,
    }
}

/// A session asked to leave nothing behind (`--no-session`, `-s -`).
pub(super) fn ephemeral(flags: &super::AgentFlags) -> bool {
    flags.no_session || flags.session_name.as_deref() == Some("-")
}

/// The key a one-shot (`-m`) session runs as, as `run_agent_session`
/// names it: `cli:<name>`, `cli:default` without `-s`.
pub(super) fn one_shot_key(session_name: Option<&str>) -> String {
    let name = session_name.unwrap_or("default");
    crate::domain::session_identity::SessionIdentity::named_cli(name)
        .map(|identity| identity.runtime_key().to_owned())
        .unwrap_or_default()
}

/// A key for a session without one: unique to this process and start.
fn unkeyed() -> String {
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("unkeyed-{}-{started}", std::process::id())
}

/// Give `agent` its audit log when it keeps one; a failure to open is a
/// warning, never a failed start.
pub(super) fn attach(
    agent: &mut AgentLoopImpl,
    base_dir: &Path,
    flags: &super::AgentFlags,
    session_key: &str,
    event_log: bool,
    stderr: &mut String,
) {
    let Some(key) = log_key(flags.workflow, ephemeral(flags), session_key, event_log) else {
        return;
    };
    match crate::infrastructure::persistence::audit_log::AuditLog::open_sync(base_dir, &key) {
        Ok(log) => agent.set_audit_log(Some(
            Arc::new(log.with_parent(flags.parent_id.clone())) as Arc<dyn AuditSink>
        )),
        Err(e) => stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n")),
    }
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;

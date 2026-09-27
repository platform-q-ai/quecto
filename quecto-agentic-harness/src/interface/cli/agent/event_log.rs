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

/// Give `agent` its audit log when it keeps one, and prepare the crash
/// record a fatal panic leaves (#2192): beside the log, for every session
/// that may leave something behind, with the log's crash line when it keeps
/// one. The record is armed only once the session is claimed. A failure to
/// open either is a warning, never a failed start.
pub(super) fn attach(
    agent: &mut AgentLoopImpl,
    base_dir: &Path,
    flags: &super::AgentFlags,
    session_key: &str,
    event_log: bool,
    stderr: &mut String,
) {
    use crate::infrastructure::persistence::audit_log::AuditLog;
    use crate::infrastructure::persistence::crash_record::{CrashTarget, prepare};
    let crash_line = match log_key(flags.workflow, ephemeral(flags), session_key, event_log) {
        Some(key) => match AuditLog::open_sync(base_dir, &key) {
            Ok(log) => {
                let log = log.with_parent(flags.parent_id.clone());
                let crash_line = log.crash_line();
                agent.set_audit_log(Some(Arc::new(log) as Arc<dyn AuditSink>));
                crash_line
            }
            Err(e) => {
                stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
                None
            }
        },
        None => None,
    };
    let keeps_record = match (ephemeral(flags), session_key.is_empty()) {
        (false, false) => Some(session_key.to_string()),
        (true, _) | (false, true) => None,
    };
    prepare(CrashTarget {
        base_dir: Some(base_dir.to_path_buf()),
        session_key: keeps_record,
        event_log: crash_line,
    });
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;

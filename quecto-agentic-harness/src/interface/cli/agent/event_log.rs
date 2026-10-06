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
    crate::domain::sessions::entities::session_identity::SessionIdentity::named_cli(name)
        .map(|identity| identity.runtime_key().to_owned())
        .unwrap_or_default()
}

/// Whether the event log is on (#2150): the loaded configuration
/// switches it on (`config_on`), or the owner's global config does.
pub(super) fn switched_on(config_on: bool, base_dir: &Path) -> bool {
    config_on || crate::infrastructure::config::telemetry::globally_enabled(base_dir)
}

/// The event-log switch decided before admission (#2313), so the
/// admission's own board calls are measured and recorded exactly when the
/// log is on: from the configuration the agent's build loads, over the same
/// layers and `QUECTO_*` overrides, without asking to trust an overlay (an
/// overlay not trusted yet is not applied, so this is never on where the
/// build's decision is off). A configuration that does not load is off.
pub(super) fn decided_before_admission(
    ctx: &crate::interface::cli::CliContext,
    flags: &super::AgentFlags,
) -> bool {
    let env_overrides = crate::interface::cli::config_loading::quecto_env_overrides();
    decided_with(ctx, flags, &env_overrides)
}

/// [`decided_before_admission`] over `env_overrides`, through the loader
/// the build itself uses (`loaded_config::load`, #2313 review L5). A
/// configuration that does not load is off, even where the global config
/// switches the log on: the build refuses to start then, and no log opens.
pub(super) fn decided_with(
    ctx: &crate::interface::cli::CliContext,
    flags: &super::AgentFlags,
    env_overrides: &std::collections::HashMap<String, String>,
) -> bool {
    let base_dir = ctx.base_dir();
    let loaded = ctx.config_selection().ok().and_then(|selection| {
        super::loaded_config::load(&base_dir, &selection, flags, false, env_overrides).ok()
    });
    match loaded {
        Some(loaded) => switched_on(loaded.config.telemetry.event_log.enabled, &base_dir),
        None => false,
    }
}

/// The session switched to `session_key` (#2192): the crash target
/// follows it, and the event log with it, when the log was the departing
/// session's own; the process's swarm board then records in the arriving
/// session's log too (#2313). Answers the log the agent writes to from
/// now on.
pub(in crate::interface::cli) fn follow_session(
    session_key: Option<&str>,
) -> Option<Arc<dyn AuditSink>> {
    let log = crate::infrastructure::persistence::crash_record::follow(session_key)?;
    if let Some(board) = crate::infrastructure::tools::swarm_bridge::process_board() {
        board.follow_session(&log);
    }
    Some(Arc::new(log))
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
    use crate::infrastructure::persistence::crash_record::{CrashTarget, prepare};
    let board = crate::infrastructure::tools::swarm_bridge::process_board();
    let target = SessionLogTarget {
        base_dir,
        session_key,
        event_log,
    };
    let crash_line = attach_to(board, agent, &target, flags, stderr);
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

/// Where a session's audit log is kept: under `base_dir`, for the session
/// `session_key`, with the event log on or off.
pub(super) struct SessionLogTarget<'a> {
    pub(super) base_dir: &'a Path,
    pub(super) session_key: &'a str,
    pub(super) event_log: bool,
}

/// [`attach`]'s log, without the crash record: gives `agent` its audit log
/// when it keeps one, and records `board`'s calls there while the event log
/// is on (what it held since admission first); answers the log's crash
/// line.
pub(super) fn attach_to(
    board: Option<&crate::infrastructure::tools::swarm_bridge::SwarmBoard>,
    agent: &mut AgentLoopImpl,
    target: &SessionLogTarget<'_>,
    flags: &super::AgentFlags,
    stderr: &mut String,
) -> Option<crate::infrastructure::persistence::audit_log::AuditCrashLine> {
    use crate::infrastructure::persistence::audit_log::AuditLog;
    let SessionLogTarget {
        base_dir,
        session_key,
        event_log,
    } = *target;
    match log_key(flags.workflow, ephemeral(flags), session_key, event_log) {
        Some(key) => match AuditLog::open_sync(base_dir, &key) {
            Ok(log) => {
                let log = log.with_parent(flags.parent_id.clone());
                let crash_line = log.crash_line();
                // The board calls this process makes are recorded in the
                // same log, while the event log is on (#2278, #2303).
                if let Some(board) = board {
                    board.record_in_session(event_log, &log);
                }
                agent.set_audit_log(Some(Arc::new(log) as Arc<dyn AuditSink>));
                crash_line
            }
            Err(e) => {
                stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
                if let Some(board) = board {
                    board.stop_recording();
                }
                None
            }
        },
        // No log: the board records nothing, not even what it held since
        // admission (#2313).
        None => {
            if let Some(board) = board {
                board.stop_recording();
            }
            None
        }
    }
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;

//! Which agents keep the audit log (#2150): a workflow session with a key,
//! as before; with `telemetry.event_log` on, every agent except one asked
//! to leave nothing behind (`-s -`, `--no-session`).
use std::path::Path;
use std::sync::Arc;

use crate::application::agent_turn::agent_loop::AgentLoopImpl;
use crate::application::audit::ports::AuditSink;
use crate::infrastructure::observability::audit_log::AuditLog;
use crate::interface::cli::board::BoardToolRecordingBuilder;
use crate::interface::tools::project_board::session_recording::SessionRecording;

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
/// session's log too (#2313), and so does the project board tool (epic
/// #2481 T3). Answers the log the agent writes to from now on.
pub(in crate::interface::cli) fn follow_session(
    session_key: Option<&str>,
) -> Option<Arc<dyn AuditSink>> {
    let log = crate::infrastructure::persistence::file::crash_record::follow(session_key)?;
    let board = crate::infrastructure::tools::swarm::swarm_bridge::process_board();
    Some(followed(log, board, BoardToolSession::process()))
}

/// [`follow_session`] once the crash target gave the arriving session's
/// `log`: `board` (the swarm's) and `board_tool` record there from now on.
pub(super) fn followed(
    log: AuditLog,
    board: Option<&crate::infrastructure::tools::swarm::swarm_bridge::SwarmBoard>,
    board_tool: &BoardToolSession,
) -> Arc<dyn AuditSink> {
    if let Some(board) = board {
        board.follow_session(&log);
    }
    board_tool.follow(&log);
    Arc::new(log)
}

/// The project board tool's recording (epic #2481 T3) and how it is built
/// again into another session's log: composition's builder, with the
/// switch the session decided when it started.
pub(in crate::interface::cli) struct BoardToolSession {
    recordings: Arc<SessionRecording>,
    source: std::sync::Mutex<Option<BoardToolSource>>,
}

/// How a bound recording is built: by `build`, with the session's switch.
#[derive(Clone, Copy)]
struct BoardToolSource {
    build: BoardToolRecordingBuilder,
    event_log: bool,
}

impl BoardToolSession {
    /// The board tool's recording `recordings`, bound to no session yet.
    pub(super) fn new(recordings: Arc<SessionRecording>) -> Self {
        Self {
            recordings,
            source: std::sync::Mutex::default(),
        }
    }

    /// The process's: its board tool's ([`SessionRecording::process`]).
    fn process() -> &'static Self {
        static PROCESS: std::sync::OnceLock<BoardToolSession> = std::sync::OnceLock::new();
        PROCESS.get_or_init(|| Self::new(SessionRecording::process()))
    }

    fn source(&self) -> std::sync::MutexGuard<'_, Option<BoardToolSource>> {
        self.source
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records the tool's calls in `log`, the session's event log, as
    /// `build` builds the recording with the session's switch `event_log`;
    /// nothing without a builder, or while the switch is off.
    fn bind(&self, build: Option<BoardToolRecordingBuilder>, event_log: bool, log: &AuditLog) {
        let mut source = self.source();
        *source = build.map(|build| BoardToolSource { build, event_log });
        let recording = source.and_then(|source| (source.build.0)(source.event_log, log));
        self.recordings.bind(recording);
    }

    /// Records nothing from now on.
    fn stop(&self) {
        let mut source = self.source();
        *source = None;
        self.recordings.stop();
    }

    /// The session switched to the one whose log is `log`: a recording
    /// bound is built again there, with the switch its session decided.
    fn follow(&self, log: &AuditLog) {
        let source = *self.source();
        if let Some(source) = source {
            self.recordings
                .follow(|| (source.build.0)(source.event_log, log));
        }
    }
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
/// open either is a warning, never a failed start. The process's project
/// board tool records in the log too (epic #2481 T3).
pub(super) fn attach(
    agent: &mut AgentLoopImpl,
    flags: &super::AgentFlags,
    target: &SessionLogTarget<'_>,
    stderr: &mut String,
) {
    use crate::infrastructure::persistence::file::crash_record::{CrashTarget, prepare};
    let board = crate::infrastructure::tools::swarm::swarm_bridge::process_board();
    let board_tool = BoardToolSession::process();
    let crash_line = attach_to(board, board_tool, agent, target, flags, stderr);
    let SessionLogTarget {
        base_dir,
        session_key,
        ..
    } = *target;
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
/// `session_key`, with the event log on or off; and how the project board
/// tool's recording is built (`None`: the tool records nothing).
pub(super) struct SessionLogTarget<'a> {
    pub(super) base_dir: &'a Path,
    pub(super) session_key: &'a str,
    pub(super) event_log: bool,
    pub(super) project_board: Option<BoardToolRecordingBuilder>,
}

/// [`attach`]'s log, without the crash record: gives `agent` its audit log
/// when it keeps one, and records `board`'s calls there while the event log
/// is on (what it held since admission first), and binds `board_tool`'s
/// recording to it; answers the log's crash line.
pub(super) fn attach_to(
    board: Option<&crate::infrastructure::tools::swarm::swarm_bridge::SwarmBoard>,
    board_tool: &BoardToolSession,
    agent: &mut AgentLoopImpl,
    target: &SessionLogTarget<'_>,
    flags: &super::AgentFlags,
    stderr: &mut String,
) -> Option<crate::infrastructure::observability::audit_log::AuditCrashLine> {
    let SessionLogTarget {
        base_dir,
        session_key,
        event_log,
        project_board,
    } = *target;
    let stop_board_tool = || board_tool.stop();
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
                // So are the board tool's, with the session's switch
                // (#2150), decided now.
                board_tool.bind(project_board, event_log, &log);
                agent.set_audit_log(Some(Arc::new(log) as Arc<dyn AuditSink>));
                crash_line
            }
            Err(e) => {
                stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
                if let Some(board) = board {
                    board.stop_recording();
                }
                stop_board_tool();
                None
            }
        },
        // No log: the board records nothing, not even what it held since
        // admission (#2313).
        None => {
            if let Some(board) = board {
                board.stop_recording();
            }
            stop_board_tool();
            None
        }
    }
}

#[cfg(test)]
#[path = "event_log_tests.rs"]
mod tests;

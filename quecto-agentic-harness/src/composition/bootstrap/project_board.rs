//! The project board's operations for one run (#2477): the git store on
//! the repository's board remote, writing as its configured committer (a
//! read needs none); the SQLite index of the board in the repository's
//! git directory; the clock referenced to the remote's HTTPS `Date`
//! (#2491); and pull request states read with `gh` when the remote is on
//! GitHub or the configured enterprise host. Built for `quecto board`,
//! which runs on the calling thread (it blocks).
//!
//! Also where a board request's recording is built (epic #2481 T2):
//! [`session_board_recording`] for the agent tool, into its session's
//! event log, and [`cli_board_recording`] for `quecto board`, into the
//! daily `board-cli:<UTC date>` log. Each derives the switch itself, with
//! the sessions' rule (#2150), and gives `None` while it is off, so no log
//! is opened and nothing is written or traced.
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use crate::application::project_board::operations::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, PullRequestStates, PullRequestStatesError,
};
use crate::application::project_board::telemetry::{BoardRecording, RecordingSwitch};
use crate::application::project_board::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    RunBoardOperation, UpdateDraftTask,
};
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::telemetry::{TelemetryConfig, globally_enabled};
use crate::infrastructure::persistence::audit_log::{AuditCrashLine, AuditLog};
use crate::infrastructure::persistence::project_board::gh_pull_requests::{
    GhPullRequestStates, github_repository,
};
use crate::infrastructure::persistence::project_board::git::{Git, GitEnvironment, GitError};
use crate::infrastructure::persistence::project_board::index::BoardIndex;
use crate::infrastructure::persistence::project_board::indexed_listings::IndexedBoardListings;
use crate::infrastructure::persistence::project_board::op_log::{
    EventLogProjectBoardOps, TracingProjectBoardOps,
};
use crate::infrastructure::persistence::project_board::store::GitBoardStore;
use crate::infrastructure::time::remote_time::ReferencedBoardClock;
use crate::interface::cli::board::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles,
};

/// # Errors
/// A write with no committer configured, or one git would not record as
/// given; no git, a repository git cannot run in, no such remote, an
/// index whose directory cannot be named (each with its cause). A write
/// warns on standard error of a malformed git command-scope set, which it
/// drops, or of one naming half an identity.
pub fn build_project_board_handles(
    environment: &ProjectBoardEnvironment,
) -> Result<ProjectBoardHandles, BuildError> {
    build_with_warnings(environment, &mut |line| eprintln!("{line}"))
}

/// [`build_project_board_handles`], telling `warn` each line to warn of: a
/// write's malformed git command-scope set, dropped, and why, or one
/// naming half an identity.
fn build_with_warnings(
    environment: &ProjectBoardEnvironment,
    warn: &mut dyn FnMut(&str),
) -> Result<ProjectBoardHandles, BuildError> {
    let unavailable = |why: String| BuildError::Unavailable(why);
    let vars = GitEnvironment::allowlisted(environment.vars.iter().cloned());
    // Only a write names a committer, so only a write is told what the
    // set naming one warns of. A warning names variables and the
    // identity's keys alone; it is redacted all the same, in case one ever
    // quotes more.
    match (environment.access, vars.command_scope_warning()) {
        (Access::Write, Some(warning)) => warn(&redact_secrets(&redact_url_userinfo(&format!(
            "warning: project board: {warning}"
        )))),
        (Access::Write | Access::Read, None) | (Access::Read, Some(_)) => {}
    }
    let git = match environment.access {
        Access::Write => Git::configured_committer(&environment.repo, &vars)
            .and_then(|committer| Git::new(&environment.repo, vars, committer))
            .map_err(write_refusal)?,
        Access::Read => Git::reader(&environment.repo, vars)
            .map_err(|error| unavailable(format!("git cannot run here: {error}")))?,
    };
    let store = Arc::new(
        GitBoardStore::new(git, &environment.remote)
            .map_err(|error| unavailable(store_reason(&error)))?
            .with_cancel(environment.cancelled.clone()),
    );
    let url = store
        .remote_url()
        .map_err(|error| unavailable(store_reason(&error)))?;
    let clock: Arc<dyn BoardClock> = Arc::new(ReferencedBoardClock::for_remote(&url));
    let index = BoardIndex::of(&store).map_err(|error| {
        unavailable(format!(
            "the board index's directory cannot be named: {error}"
        ))
    })?;
    let listings = Arc::new(IndexedBoardListings::new(
        index,
        Arc::clone(&store) as _,
        Arc::clone(&store) as _,
    ));
    let prs = pull_request_states(&url, &environment.vars);
    let store: Arc<dyn ProjectBoardStore> = store;
    let built = |error: crate::application::project_board::dto::ProjectBoardError| {
        unavailable(error.to_string())
    };
    let complete = match prs {
        Ok(prs) => Ok(
            CompleteMergedTask::new(Arc::clone(&store), Arc::clone(&clock), prs).map_err(built)?,
        ),
        Err(why) => Err(why),
    };
    let parts = BoardOperationParts {
        bootstrap: BootstrapProjectBoard::new(Arc::clone(&store)),
        create: CreateBoardTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        update: UpdateDraftTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        change: ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        complete,
        clock,
        listings,
    };
    Ok(ProjectBoardHandles {
        operations: RunBoardOperation::new(parts),
    })
}

/// Pull request states of the remote at `url`, read with `gh` when it is
/// on GitHub or the configured enterprise host (`GH_HOST` in `vars`); or
/// why none are.
fn pull_request_states(
    url: &str,
    vars: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Arc<dyn PullRequestStates>, NoPullRequestHost> {
    let configured_host = vars
        .iter()
        .find(|(key, _)| key == "GH_HOST")
        .and_then(|(_, value)| value.to_str());
    match github_repository(url, configured_host) {
        Some(repo) => GhPullRequestStates::new(&repo, vars.iter().cloned())
            .map(|states| Arc::new(states) as _)
            .map_err(|error| match error {
                PullRequestStatesError::Unavailable(why) => NoPullRequestHost::Unreachable(why),
            }),
        None => Err(NoPullRequestHost::NotAPullRequestHost),
    }
}

/// Why a write's store cannot be built: an identity problem (none
/// configured, or one git would not record as given) is the committer's;
/// anything else (no git, a timeout, a repository git cannot run in) is
/// the store's, with its cause.
fn write_refusal(error: GitError) -> BuildError {
    match error {
        GitError::Identity(why) => BuildError::NoCommitter(why),
        unavailable @ (GitError::NotFound
        | GitError::Spawn(_)
        | GitError::TimedOut { .. }
        | GitError::TooMuchOutput { .. }
        | GitError::Failed { .. }) => {
            BuildError::Unavailable(format!("git cannot run here: {unavailable}"))
        }
    }
}

/// A store's refusal in its own words.
fn store_reason(error: &BoardStoreError) -> String {
    match error {
        BoardStoreError::NoBoard => "the project has no board yet".into(),
        BoardStoreError::Clash(why)
        | BoardStoreError::InvalidWrite(why)
        | BoardStoreError::Unavailable(why) => why.clone(),
    }
}

/// The agent tool's recording, into its session's event log `log`: only
/// while `telemetry` (the session's configuration) or the owner's global
/// file at `base_dir` switches the event log on, and only when `log` has a
/// synchronous line to append through. `None` otherwise.
pub fn session_board_recording(
    telemetry: &TelemetryConfig,
    base_dir: &Path,
    log: &AuditLog,
) -> Option<BoardRecording> {
    recording(switched_on(telemetry.event_log.enabled, base_dir), || {
        log.crash_line()
    })
}

/// The `quecto board` CLI's recording, into today's `board-cli:<UTC
/// date>` log under `base_dir` (`audit/board-cli_<date>.jsonl`). The CLI
/// loads no layered configuration, so it follows the owner's global file
/// alone. `None` while that is off, or when the log cannot be opened.
pub fn cli_board_recording(base_dir: &Path) -> Option<BoardRecording> {
    recording(switched_on(false, base_dir), || {
        let key = cli_log_key(SystemTime::now())?;
        AuditLog::open_sync(base_dir, &key).ok()?.crash_line()
    })
}

/// The sessions' switch (#2150, `switched_on` in
/// `interface/cli/agent/event_log.rs`): the loaded configuration's, or the
/// owner's global one, so a repository config switches it on, never off.
fn switched_on(config_on: bool, base_dir: &Path) -> RecordingSwitch {
    RecordingSwitch::resolved(config_on || globally_enabled(base_dir))
}

/// A recording into the log on the line `open` gives, traced; `open` is
/// not called while `switch` is off.
fn recording(
    switch: RecordingSwitch,
    open: impl FnOnce() -> Option<AuditCrashLine>,
) -> Option<BoardRecording> {
    BoardRecording::when(switch, || {
        let ops = TracingProjectBoardOps::over(EventLogProjectBoardOps::new(open()?));
        Some(Arc::new(ops) as _)
    })
}

/// The CLI's daily log key at `now`: `board-cli:<YYYY-MM-DD>`, the UTC
/// date; `None` for a time with no such date.
fn cli_log_key(now: SystemTime) -> Option<String> {
    let stamp = humantime::format_rfc3339_seconds(now).to_string();
    let day = stamp.get(..10)?;
    let shaped = day.char_indices().all(|(at, c)| match at {
        4 | 7 => c == '-',
        _ => c.is_ascii_digit(),
    });
    shaped.then(|| format!("board-cli:{day}"))
}

#[cfg(test)]
#[path = "project_board_tests.rs"]
mod tests;

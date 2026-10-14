//! The project board's operations for one run (#2477): the git store on
//! the repository's board remote, writing as its configured committer (a
//! read needs none); the SQLite index of the board in the repository's
//! git directory; the clock referenced to the remote's HTTPS `Date`
//! (#2491); and pull request states read with `gh` when the remote is on
//! GitHub or the configured enterprise host. Built for `quecto board`,
//! which runs on the calling thread (it blocks).
//!
//! Also where a board request's recording is built (epic #2481 T2):
//! [`session_board_recording_decided`] for the agent tool, into its
//! session's event log with the switch the session decided (T3), and
//! [`cli_board_recording`] for `quecto board`, into the daily
//! `board-cli:<UTC date>` log, deriving the switch from the owner's global
//! file. Each gives `None` while it is off, so no log is opened and nothing
//! is written or traced.
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use crate::application::project_board::operations::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, PullRequestStates, PullRequestStatesError,
};
use crate::application::project_board::telemetry::{BoardRecording, RecordingSwitch};
use crate::application::project_board::use_cases::{
    AcceptPlan, BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask,
    CreateBoardTask, DiscardPlan, EditPlan, EndRun, ProposePlan, QueryBoard, RunBoardOperation,
    UpdateDraftTask,
};
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::config::telemetry::globally_enabled;
use crate::infrastructure::persistence::audit_log::AuditLog;
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
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
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
        propose_plan: ProposePlan::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        edit_plan: EditPlan::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        accept_plan: AcceptPlan::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        discard_plan: DiscardPlan::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        // No front end of this slice sees a run's swarm (Q8b's tool does).
        end_run: EndRun::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            Arc::new(MarkdownTextCells),
            None,
        )
        .map_err(built)?,
        clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock)).map_err(built)?,
        complete,
        clock,
        listings,
        query: QueryBoard::new(store),
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
        | BoardStoreError::WriteTooLarge(why)
        | BoardStoreError::Unavailable(why)
        | BoardStoreError::TooLarge(why)
        | BoardStoreError::FolderSetAside(why) => why.clone(),
    }
}

/// The agent tool's recording, into its session's event log `log`: only
/// while the switch its session decided when it started (`event_log_on`,
/// with the sessions' rule #2150: the session's configuration or the
/// owner's global file, `switched_on` in `interface/cli/agent/event_log.rs`)
/// is on, and only when `log` has a synchronous line to append through.
/// `None` otherwise. Its records follow a session switch with that switch,
/// so a later edit to a config file neither starts nor stops them, as the
/// swarm board's records follow (`record_in_session(true, …)`; epic #2481
/// T3).
pub fn session_board_recording_decided(
    event_log_on: bool,
    log: &AuditLog,
) -> Option<BoardRecording> {
    session_recording(RecordingSwitch::resolved(event_log_on), log)
}

/// A session's recording into `log`, only while `switch` is on and `log`
/// has a synchronous line to append through.
fn session_recording(switch: RecordingSwitch, log: &AuditLog) -> Option<BoardRecording> {
    BoardRecording::when(switch, || {
        let ops = EventLogProjectBoardOps::new(log.crash_line()?);
        Some(Arc::new(TracingProjectBoardOps::over(ops)) as _)
    })
}

/// The `quecto board` CLI's recording, into the daily `board-cli:<UTC
/// date>` log under `base_dir` (`audit/board-cli_<date>.jsonl`). The CLI
/// loads no layered configuration, so it follows the owner's global file
/// alone; `None` while that is off. The log is opened, and dated, by the
/// first record, so a run that records nothing leaves no file; a log that
/// cannot be opened then records nothing, to the log or to `tracing`,
/// with one warning. The daily files are kept with no retention yet: each
/// stops at the event log's cap, which is only approximate here, since
/// each CLI process counts from the file's size when it opened it, so
/// processes writing at once can overshoot the cap or write a line after
/// its `log_capped` record (a reader, T8, must tolerate both).
pub fn cli_board_recording(base_dir: &Path) -> Option<BoardRecording> {
    let base_dir = base_dir.to_path_buf();
    let switch = RecordingSwitch::resolved(globally_enabled(base_dir.as_path()));
    BoardRecording::when(switch, move || {
        let ops = EventLogProjectBoardOps::opened_by_first_record(move || {
            let key = cli_log_key(SystemTime::now())?;
            AuditLog::open_sync(&base_dir, &key).ok()?.crash_line()
        });
        Some(Arc::new(TracingProjectBoardOps::over(ops)) as _)
    })
}

/// The first second of year 10000: RFC 3339 dates have four-digit years.
const YEAR_10000_UNIX_SECS: u64 = 253_402_300_800;

/// The CLI's daily log key at `now`: `board-cli:<YYYY-MM-DD>`, the UTC
/// date; `None` for a time with no such date (before the epoch, or in
/// year 10000 or later), never a panic.
fn cli_log_key(now: SystemTime) -> Option<String> {
    now.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .filter(|since| since.as_secs() < YEAR_10000_UNIX_SECS)?;
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

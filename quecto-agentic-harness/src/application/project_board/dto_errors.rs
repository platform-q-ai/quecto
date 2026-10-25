//! How a board operation's refusal reads ([`ProjectBoardError`]'s
//! words), moved beside `dto.rs` unchanged as it grew past its room.
use std::fmt;

use super::{IdUser, ProjectBoardError, RunAfterUndo};
use crate::application::project_board::ports::{
    BoardClockError, BoardStoreError, PullRequestStatesError,
};

impl fmt::Display for ProjectBoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(BoardStoreError::NoBoard) => f.write_str("the project has no board yet"),
            Self::Store(BoardStoreError::Clash(why)) => {
                write!(f, "the board clashes with the repository: {why}")
            }
            Self::Store(
                BoardStoreError::InvalidWrite(why) | BoardStoreError::WriteTooLarge(why),
            ) => {
                write!(f, "the board refused an invalid write: {why}")
            }
            Self::Store(BoardStoreError::Unavailable(why)) => {
                write!(f, "the board store is unavailable: {why}")
            }
            Self::Store(BoardStoreError::TooLarge(why)) => {
                write!(f, "the board is too large to read: {why}")
            }
            Self::Store(BoardStoreError::FolderSetAside { why, .. }) => f.write_str(why),
            Self::Store(BoardStoreError::Moved(moved)) => write!(f, "{moved}"),
            Self::Store(BoardStoreError::MovedMalformed(why)) => f.write_str(why),
            Self::Rule(error) => write!(f, "{error}"),
            Self::Clock(BoardClockError::OutOfRange(read)) => {
                write!(
                    f,
                    "this clock reads {read}, a time the board cannot spell; fix the clock"
                )
            }
            Self::Invalid(error) => write!(f, "invalid task: {error}"),
            Self::NotFound(id) => write!(f, "no task {}", id.as_str()),
            Self::Exists(id) => write!(f, "task {} already exists", id.as_str()),
            // Owner-approved wording (#2481 S7).
            Self::IdInUse { id, by } => {
                let name = id.as_str();
                write!(f, "no task {name} is on the board, but ")?;
                match by {
                    IdUser::Discussion(discussion) => {
                        write!(f, "discussion {} is about {name}", discussion.as_str())?;
                    }
                    IdUser::Plan(_) | IdUser::Run(_) | IdUser::Change(_) | IdUser::Review(_) => {
                        write!(f, "{} is still there", by.key(id))?;
                    }
                }
                write!(f, " and uses that id; choose another id")
            }
            Self::SetAsideFile { path, object } => write!(
                f,
                "the file {path:?} is set aside as unreadable and its task is unknown; mend it, or \
                 discard it with clear_set_aside(path \"{path}\", object \"{object}\")"
            ),
            Self::Duplicate { key } => write!(
                f,
                "{key} is on the board more than once; its store must be mended"
            ),
            Self::Quarantined { key, path, object } => write!(
                f,
                "{key} is set aside as unreadable; mend its file, or discard it with \
                 clear_set_aside(path \"{path}\", object \"{object}\")"
            ),
            Self::UnknownReference(id) => {
                write!(f, "no task {} on the board to refer to", id.as_str())
            }
            Self::ClosesCycle { id, by } => write!(
                f,
                "task {id} would close a cycle: it leads to task {by}, which already names {id} \
                 as its parent or a dependency",
                id = id.as_str(),
                by = by.as_str()
            ),
            Self::InvalidWriter(error) => write!(f, "the board's writer is invalid: {error}"),
            Self::PrStateUnknown(pr) => write!(f, "pull request {pr} is not known to the host"),
            Self::PullRequests(PullRequestStatesError::Unavailable(why)) => {
                write!(f, "the pull requests' states are unavailable: {why}")
            }
            Self::TaskAhead { id, updated } => write!(
                f,
                "task {} was stamped {} by a clock that ran ahead",
                id.as_str(),
                updated.as_str()
            ),
            Self::ClockOutOfRange(reference) => write!(
                f,
                "the reference time {} is out of range",
                reference.as_str()
            ),
            Self::ClockAhead { now, reference } => write!(
                f,
                "this clock reads {} but the board's reference reads {}; fix the clock",
                now.as_str(),
                reference.as_str()
            ),
            Self::Contended { attempts } => {
                write!(f, "the board kept moving: {attempts} attempts lost")
            }
            Self::Refused(why) => write!(f, "the board refused the write: {why}"),
            Self::NoPullRequestHost(why) => write!(f, "{why}"),
            Self::NotSetAside { path, object } => {
                write!(f, "no file is set aside at {path:?} as {object:?}")
            }
            Self::Stale {
                id,
                expected,
                updated,
            } => write!(
                f,
                "task {} changed since it was read (updated {}, not {}); read it again",
                id.as_str(),
                updated.as_str(),
                expected.as_str()
            ),
            Self::NoLauncher => f.write_str(
                "this session may not spawn, so it cannot launch a coordinator: \
                 start the task from a session that may",
            ),
            Self::Brief { reason, .. } => write!(f, "the plan cannot brief a swarm: {reason}"),
            Self::RunLive { task, run, agent } => write!(
                f,
                "run {run} of {task} still has its swarm running in this session: stop its \
                 coordinator first (agent_cmd kill {agent}), then end the run",
                task = task.as_str()
            ),
            Self::LaunchFailed {
                reason,
                run,
                run_state,
                released: _,
                still_running,
            } => {
                write!(f, "the coordinator did not launch: {reason}; ")?;
                if let Some(agent) = still_running {
                    // Nothing was undone: the coordinator may still run.
                    return write!(
                        f,
                        "coordinator {agent} may still be running, so run {run} is left open \
                         and you still hold the task: stop it with agent_cmd kill {agent}, \
                         then end the run with end_run(task, run {run}, outcome failed)"
                    );
                }
                // The run's end moves its task (#2705 T5): nothing else.
                match run_state {
                    RunAfterUndo::Folded => write!(
                        f,
                        "run {run} was folded back as failed, and the task is ready again"
                    ),
                    RunAfterUndo::EndedAlready => {
                        write!(f, "run {run} had already ended, and its end moved the task")
                    }
                    RunAfterUndo::Open => write!(
                        f,
                        "run {run} could not be folded back: it is still open, and you still \
                         hold the task; end it with end_run(task, run {run}, outcome failed)"
                    ),
                }
            }
        }
    }
}

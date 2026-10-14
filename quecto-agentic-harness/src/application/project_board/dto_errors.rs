//! How a board operation's refusal reads ([`ProjectBoardError`]'s
//! words), moved beside `dto.rs` unchanged as it grew past its room.
use std::fmt;

use super::{IdUser, ProjectBoardError, RunAfterUndo};
use crate::application::project_board::ports::{
    ArtifactKey, BoardClockError, BoardStoreError, PullRequestStatesError,
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
            Self::Store(BoardStoreError::FolderSetAside(why)) => f.write_str(why),
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
                "the file {path:?} (object {object}) is set aside as unreadable and its task is \
                 unknown; mend or clear it"
            ),
            Self::Duplicate { key } => write!(
                f,
                "{key} is on the board more than once; its store must be mended"
            ),
            // A task's message is the one agents already know.
            Self::Quarantined {
                key: key @ ArtifactKey::Task(_),
                ..
            } => write!(f, "{key} is set aside as unreadable; mend its file"),
            Self::Quarantined { key, path } => {
                write!(f, "{key} is set aside as unreadable; mend or clear {path}")
            }
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
            Self::LaunchFailed {
                reason,
                run,
                run_state,
                released,
            } => {
                write!(f, "the coordinator did not launch: {reason}; ")?;
                match run_state {
                    RunAfterUndo::Folded => write!(f, "run {run} was folded back as failed")?,
                    RunAfterUndo::EndedAlready => write!(f, "run {run} had already ended")?,
                    RunAfterUndo::Open => write!(
                        f,
                        "run {run} could not be folded back: it is still open; end it with \
                         end_run(task, run {run}, outcome failed)"
                    )?,
                }
                match released {
                    Some(true) => f.write_str(", and the task is ready again"),
                    Some(false) => f.write_str(
                        ", but the task could not be moved back to ready: you still hold it",
                    ),
                    None => f.write_str(", and you still hold the task"),
                }
            }
        }
    }
}

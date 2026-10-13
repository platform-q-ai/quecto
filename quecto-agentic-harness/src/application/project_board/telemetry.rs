//! Board op telemetry in the application (epic #2481, T1): the op kind of
//! a request, and whether its `board_op` record is written at all
//! ([`BoardRecording`]).
use std::sync::Arc;

use super::dto::BoardOp;
use super::operations::BoardRequest;
use super::ports::ProjectBoardOpLog;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{Access, BoardOpRecord, OpKind};

/// The op `request` is. Exhaustive with no catch-all: a new request or
/// change op does not compile until it has its kind.
pub fn op_kind(request: &BoardRequest) -> OpKind {
    match request {
        BoardRequest::Bootstrap => OpKind::Bootstrap,
        BoardRequest::List(_) => OpKind::List,
        BoardRequest::ListReady => OpKind::ListReady,
        BoardRequest::ListTakeable => OpKind::ListTakeable,
        BoardRequest::Show(_) => OpKind::Show,
        BoardRequest::Create(_) => OpKind::Create,
        BoardRequest::UpdateDraft(_) => OpKind::UpdateDraft,
        BoardRequest::Change(_, BoardOp::Move(_)) => OpKind::Move,
        BoardRequest::Change(_, BoardOp::Renew) => OpKind::Renew,
        BoardRequest::Change(_, BoardOp::TakeOver) => OpKind::TakeOver,
        BoardRequest::Change(_, BoardOp::AddPr(_)) => OpKind::LinkPr,
        BoardRequest::Change(_, BoardOp::RemovePr(_)) => OpKind::UnlinkPr,
        BoardRequest::Complete(_) => OpKind::Complete,
        BoardRequest::Clear { .. } => OpKind::ClearSetAside,
    }
}

/// Whether the op `kind` writes the board (as [`BoardRequest::writes`]
/// says of its request) or reads it; an `unknown` op reads nothing, so it
/// is a read. Exhaustive, so a new op does not compile until it has its
/// access.
pub fn access_of(kind: OpKind) -> Access {
    match kind {
        OpKind::Bootstrap
        | OpKind::Create
        | OpKind::UpdateDraft
        | OpKind::Move
        | OpKind::Renew
        | OpKind::TakeOver
        | OpKind::LinkPr
        | OpKind::UnlinkPr
        | OpKind::Complete
        | OpKind::ClearSetAside => Access::Write,
        OpKind::List
        | OpKind::ListReady
        | OpKind::ListTakeable
        | OpKind::Show
        | OpKind::Unknown => Access::Read,
    }
}

/// The task `request` acts on, when its op names one by its `task` field.
pub fn task_of(request: &BoardRequest) -> Option<&Slug> {
    match request {
        BoardRequest::Show(id) | BoardRequest::Change(id, _) | BoardRequest::Complete(id) => {
            Some(id)
        }
        BoardRequest::Create(task) => Some(&task.id),
        BoardRequest::UpdateDraft(update) => Some(&update.id),
        BoardRequest::Bootstrap
        | BoardRequest::List(_)
        | BoardRequest::ListReady
        | BoardRequest::ListTakeable
        | BoardRequest::Clear { .. } => None,
    }
}

/// The pull request `request` links or unlinks.
pub fn pr_of(request: &BoardRequest) -> Option<u64> {
    match request {
        BoardRequest::Change(_, BoardOp::AddPr(pr) | BoardOp::RemovePr(pr)) => Some(*pr),
        BoardRequest::Change(_, BoardOp::Move(_) | BoardOp::Renew | BoardOp::TakeOver)
        | BoardRequest::Bootstrap
        | BoardRequest::List(_)
        | BoardRequest::ListReady
        | BoardRequest::ListTakeable
        | BoardRequest::Show(_)
        | BoardRequest::Create(_)
        | BoardRequest::UpdateDraft(_)
        | BoardRequest::Complete(_)
        | BoardRequest::Clear { .. } => None,
    }
}

/// Whether the event log is on, as config resolved it
/// (`telemetry.event_log.enabled`). Enforced: an architecture rule lets
/// only composition name it, and only one composition file
/// (`composition/bootstrap/project_board.rs`, T2) build it. Convention,
/// held by T2's contract tests: that file derives it with the sessions'
/// own rule (#2150), the loaded configuration's switch or the owner's
/// global one (`config_on || globally_enabled(base_dir)`), so a
/// repository's config may switch the log on, never off; the agent tool
/// passes its session's configuration, and the `quecto board` CLI, which
/// loads no layered configuration, follows the owner's global file only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingSwitch(bool);

impl RecordingSwitch {
    /// The switch as config resolved it.
    pub fn resolved(enabled: bool) -> Self {
        Self(enabled)
    }
}

/// A board request's recording: where its record goes (and, from T5, the
/// meter that times its store), held as an `Option<BoardRecording>`.
///
/// Enforced: [`Self::when`] is its one constructor and gives `None` while
/// the switch is off, without building the log; architecture rules let
/// only composition call it, name the `ProjectBoardOpLog` port only in the
/// port's own files, T2's adapter and composition, never by an alias, and
/// keep the port and T2's adapters out of every public signature in
/// composition, so every other module reaches the log only through a
/// recording. Convention, held by T2's contract tests: composition's
/// builders take the configuration (or `base_dir`), derive the switch
/// themselves, and return `Option<BoardRecording>`, `None` while the
/// switch is off; swarm's `board_op_log` (`composition/swarm.rs`) is the
/// model for that `None`, not for its return type.
pub struct BoardRecording {
    log: Arc<dyn ProjectBoardOpLog>,
}

impl BoardRecording {
    /// A recording into the log `log` builds, only while `switch` is on;
    /// `log` is not called while it is off, and may itself find no log to
    /// write to.
    pub fn when(
        switch: RecordingSwitch,
        log: impl FnOnce() -> Option<Arc<dyn ProjectBoardOpLog>>,
    ) -> Option<Self> {
        match switch {
            RecordingSwitch(true) => log().map(|log| Self { log }),
            RecordingSwitch(false) => None,
        }
    }

    /// Writes `record` to the log: synchronously, never panicking.
    pub fn record(&self, record: BoardOpRecord) {
        self.log.record(record);
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;

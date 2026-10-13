//! Board op telemetry in the application (epic #2481, T1): the op kind of
//! a request, and whether its `board_op` record is written at all
//! ([`BoardRecording`]); T4: what the use cases saw of it ([`OpFacts`]).
use std::sync::Arc;

use super::dto::BoardOp;
use super::operations::{BoardRequest, Completion};
use super::ports::ProjectBoardOpLog;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, ClaimFacts, CompletionFacts, CompletionKind, Normalised, OpKind,
    StaticKind,
};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// The op `request` is. Exhaustive with no catch-all: a new request or
/// change op does not compile until it has its kind.
pub fn op_kind(request: &BoardRequest) -> OpKind {
    match request {
        BoardRequest::Bootstrap => OpKind::Bootstrap,
        BoardRequest::List(_) => OpKind::List,
        BoardRequest::ListReady => OpKind::ListReady,
        BoardRequest::ListTakeable => OpKind::ListTakeable,
        BoardRequest::Show(..) => OpKind::Show,
        BoardRequest::Create(_) => OpKind::Create,
        BoardRequest::UpdateDraft(_) => OpKind::UpdateDraft,
        BoardRequest::Change(_, BoardOp::Move(_)) => OpKind::Move,
        BoardRequest::Change(_, BoardOp::Renew) => OpKind::Renew,
        BoardRequest::Change(_, BoardOp::TakeOver) => OpKind::TakeOver,
        BoardRequest::Change(_, BoardOp::AddPr(_)) => OpKind::LinkPr,
        BoardRequest::Change(_, BoardOp::RemovePr(_)) => OpKind::UnlinkPr,
        BoardRequest::Complete(_) => OpKind::Complete,
        BoardRequest::Clear { .. } => OpKind::ClearSetAside,
        BoardRequest::ProposePlan(_) => OpKind::ProposePlan,
        BoardRequest::EditPlan(_) => OpKind::EditPlan,
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
        | OpKind::ClearSetAside
        | OpKind::ProposePlan
        | OpKind::EditPlan => Access::Write,
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
        BoardRequest::Show(id, _) | BoardRequest::Change(id, _) | BoardRequest::Complete(id) => {
            Some(id)
        }
        BoardRequest::Create(task) => Some(&task.id),
        BoardRequest::UpdateDraft(update) => Some(&update.id),
        BoardRequest::ProposePlan(proposal) => Some(&proposal.task),
        BoardRequest::EditPlan(edit) => Some(&edit.task),
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
        | BoardRequest::Show(..)
        | BoardRequest::Create(_)
        | BoardRequest::UpdateDraft(_)
        | BoardRequest::Complete(_)
        | BoardRequest::ProposePlan(_)
        | BoardRequest::EditPlan(_)
        | BoardRequest::Clear { .. } => None,
    }
}

/// What the board's use cases saw of one request (T4), for its record:
/// gathered only for a recorded call (a use case given none notes
/// nothing, and times nothing), and only ever kinds, counts and spans.
/// Each fact is `None` where it does not apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpFacts {
    /// The task's status as the change last read it.
    pub status_from: Option<TaskStatus>,
    /// A move's target, made or refused; otherwise the written task's.
    pub status_to: Option<TaskStatus>,
    /// The claim the change last found on its task.
    pub claim: Option<ClaimFacts>,
    /// The attempts the write began, and how many lost the race.
    pub attempts: Option<u64>,
    pub lost: Option<u64>,
    /// The time the write paused between attempts, when it did.
    pub pause_ms: Option<u64>,
    /// What came of completing the task.
    pub completion: Option<CompletionFacts>,
    /// What the normaliser changed in the text fields given.
    pub normalised: Option<Normalised>,
    /// The plan versions and runs a `show` gave (#2481 M4a).
    pub plans: Option<u64>,
    pub runs: Option<u64>,
    /// The plan version a `propose_plan` wrote or an `edit_plan` named,
    /// and the task items and team roles of the plan either wrote (#2481
    /// M4b).
    pub plan_version: Option<u64>,
    pub items: Option<u64>,
    pub roles: Option<u64>,
}

impl OpFacts {
    /// Writes each fact there is into `record`, leaving every field it
    /// has none for as it was. A status is recorded as its own name, a
    /// known kind.
    pub fn fill(&self, record: &mut BoardOpRecord) {
        fn put<T: Clone>(field: &mut Option<T>, fact: Option<&T>) {
            if let Some(fact) = fact {
                *field = Some(fact.clone());
            }
        }
        let status = |status: &TaskStatus| StaticKind::new(status.as_str());
        put(
            &mut record.status_from,
            self.status_from.as_ref().map(status).as_ref(),
        );
        put(
            &mut record.status_to,
            self.status_to.as_ref().map(status).as_ref(),
        );
        put(&mut record.claim, self.claim.as_ref());
        put(&mut record.attempts, self.attempts.as_ref());
        put(&mut record.lost, self.lost.as_ref());
        put(&mut record.pause_ms, self.pause_ms.as_ref());
        put(&mut record.completion, self.completion.as_ref());
        put(&mut record.normalised, self.normalised.as_ref());
        put(&mut record.plans, self.plans.as_ref());
        put(&mut record.runs, self.runs.as_ref());
        put(&mut record.plan_version, self.plan_version.as_ref());
        put(&mut record.items, self.items.as_ref());
        put(&mut record.roles, self.roles.as_ref());
    }
}

/// A completion of `kind` for a task linking `linked` pull requests,
/// `pending` of them not merged yet. A failed one's code and rule are
/// the front end's to name (its error codes are the answer's), and the
/// write's own counts its caller's to add, so they are left out here.
pub fn completion_facts(
    kind: CompletionKind,
    linked: usize,
    pending: Option<usize>,
) -> CompletionFacts {
    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
    CompletionFacts {
        completion: kind,
        completion_code: None,
        completion_rule: None,
        prs_linked: count(linked),
        prs_pending: pending.map(count),
        completion_attempts: None,
        completion_lost: None,
        completion_pause_ms: None,
    }
}

/// `completion` as a record holds it, for a task linking `linked` pull
/// requests ([`completion_facts`]).
pub fn completion_facts_of(completion: &Completion, linked: usize) -> CompletionFacts {
    match completion {
        Completion::Done(_) => completion_facts(CompletionKind::Done, linked, None),
        Completion::Pending(prs) => {
            completion_facts(CompletionKind::Pending, linked, Some(prs.len()))
        }
        Completion::Failed(_) => completion_facts(CompletionKind::Failed, linked, None),
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

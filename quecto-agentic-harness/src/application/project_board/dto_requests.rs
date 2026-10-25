//! The board's requests and outcomes as values (#2477; moved here by
//! epic #2705 B6): what a front end hands the `BoardOperations` port and
//! what it is handed back. Nothing here names a use case, so the port and
//! any adapter of it (the local one, a board server's) depend on values
//! alone; `operations` re-exports them beside the parts the use case runs.
use std::collections::BTreeMap;

use super::{
    BoardOp, DraftUpdate, EndedRun, NewBoardTask, PlanAcceptance, PlanDiscard, PlanEdit,
    PlanProposal, ProjectBoardError, RunEnd, ShownTask, TaskStart,
};
use crate::application::project_board::ports::{BoardHead, KeepCopy};
use crate::application::project_board::start::StartedTask;
use crate::application::project_board::task_summary::Listed;
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::plans::Accepted;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::FrontEnd;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// Why summaries are refused of a request that lists nothing.
pub const SUMMARIES_LISTINGS_ONLY: &str = "is taken only by list, list_ready and list_takeable";

/// Which tasks a `list` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListFilter {
    All,
    Status(TaskStatus),
    Children(Slug),
    Dependents(Slug),
}

/// One board operation, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardRequest {
    Bootstrap,
    List(ListFilter),
    ListReady,
    ListTakeable,
    /// A task with its plans and runs; or, given a page, only that page of
    /// its plan: the accepted one, or the version the view names.
    Show(Slug, PlanView),
    Create(NewBoardTask),
    UpdateDraft(DraftUpdate),
    Change(Slug, BoardOp),
    Complete(Slug),
    /// A ready or claimed task done without a run, for the reason given
    /// (#2705 T4).
    DoneWithoutRun {
        task: Slug,
        reason: String,
    },
    /// A new draft version of a task's plan (#2481 M4b).
    ProposePlan(PlanProposal),
    /// A draft plan patched: only the fields it gives change.
    EditPlan(PlanEdit),
    /// A draft plan made the task's plan (#2481 M4c).
    AcceptPlan(PlanAcceptance),
    /// A draft plan discarded; it is kept.
    DiscardPlan(PlanDiscard),
    /// A run ended by hand (S4c), for a run no swarm will fold back.
    EndRun(RunEnd),
    /// A task started on its accepted plan and its coordinator launched
    /// (S4c).
    Start(TaskStart),
    /// Clear the file set aside at `path` whose contents are `object`,
    /// resolving every name listed more than once (`repeated`).
    Clear {
        path: String,
        object: String,
        repeated: BTreeMap<String, KeepCopy>,
    },
}

impl BoardRequest {
    /// The request writes the board, so a front end builds the store with
    /// a committer for it; a read needs none.
    pub fn writes(&self) -> bool {
        match self {
            Self::Bootstrap
            | Self::Create(_)
            | Self::UpdateDraft(_)
            | Self::Change(..)
            | Self::Complete(_)
            | Self::DoneWithoutRun { .. }
            | Self::ProposePlan(_)
            | Self::EditPlan(_)
            | Self::AcceptPlan(_)
            | Self::DiscardPlan(_)
            | Self::EndRun(_)
            | Self::Start(_)
            | Self::Clear { .. } => true,
            Self::List(_) | Self::ListReady | Self::ListTakeable | Self::Show(..) => false,
        }
    }

    /// The request reads the clock: it stamps tasks with its time, or (a
    /// takeable listing) judges claims by it, so whether that time was
    /// bounded against an outside one matters.
    pub fn reads_clock(&self) -> bool {
        match self {
            Self::Create(_)
            | Self::UpdateDraft(_)
            | Self::Change(..)
            | Self::Complete(_)
            | Self::DoneWithoutRun { .. }
            | Self::ProposePlan(_)
            | Self::EditPlan(_)
            | Self::AcceptPlan(_)
            | Self::DiscardPlan(_)
            | Self::EndRun(_)
            | Self::Start(_)
            | Self::ListTakeable => true,
            Self::Bootstrap
            | Self::Clear { .. }
            | Self::List(_)
            | Self::ListReady
            | Self::Show(..) => false,
        }
    }

    /// The request lists tasks: the requests a [`ListSummaries::Wanted`]
    /// may be asked of (an allowlist).
    pub fn is_listing(&self) -> bool {
        match self {
            Self::List(_) | Self::ListReady | Self::ListTakeable => true,
            Self::Bootstrap
            | Self::Show(..)
            | Self::Create(_)
            | Self::UpdateDraft(_)
            | Self::Change(..)
            | Self::Complete(_)
            | Self::DoneWithoutRun { .. }
            | Self::ProposePlan(_)
            | Self::EditPlan(_)
            | Self::AcceptPlan(_)
            | Self::DiscardPlan(_)
            | Self::EndRun(_)
            | Self::Start(_)
            | Self::Clear { .. } => false,
        }
    }
}

/// Whether a listing reads the whole board to summarise each task it
/// lists ([`crate::application::project_board::task_summary`]). Omitted
/// by default: a listing then reads only what it lists, as ever; a front
/// end asks for summaries only of a listing ([`BoardRequest::is_listing`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListSummaries {
    #[default]
    Omitted,
    Wanted,
}

/// Which plan a `show` gives and how much of it (#2481 M4b): the version
/// named (`None`: the task's accepted plan), whole, or only the page
/// named. The default is the accepted plan, whole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanView {
    pub version: Option<u32>,
    pub page: Option<PlanPage>,
}

/// Where a `show` reads on in its task's accepted plan (#2481 M4a review
/// round 2): the approach from its character `approach_from`, the handoff
/// from its character `handoff_from` (#2705 T3), the task items from the
/// index `items_from`. A part not named is not shown, so a page names at
/// least one ([`Self::paged`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanPage {
    approach_from: Option<usize>,
    handoff_from: Option<usize>,
    items_from: Option<usize>,
}

impl PlanPage {
    /// The page from `approach_from` and `items_from`; none when neither
    /// is named.
    pub fn of(approach_from: Option<usize>, items_from: Option<usize>) -> Option<Self> {
        Self::paged(approach_from, None, items_from)
    }

    /// The page from `approach_from`, `handoff_from` and `items_from`;
    /// none when none is named.
    pub fn paged(
        approach_from: Option<usize>,
        handoff_from: Option<usize>,
        items_from: Option<usize>,
    ) -> Option<Self> {
        match (approach_from, handoff_from, items_from) {
            (None, None, None) => None,
            (Some(_), _, _) | (_, Some(_), _) | (_, _, Some(_)) => Some(Self {
                approach_from,
                handoff_from,
                items_from,
            }),
        }
    }

    pub fn approach_from(&self) -> Option<usize> {
        self.approach_from
    }

    pub fn handoff_from(&self) -> Option<usize> {
        self.handoff_from
    }

    pub fn items_from(&self) -> Option<usize> {
        self.items_from
    }
}

/// What came of completing a task after a change that may complete it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completion {
    /// Every linked pull request merged: the task is done.
    Done(Box<Task>),
    /// These linked pull requests have not merged yet; nothing is wrong.
    Pending(Vec<u64>),
    /// The completion could not be judged (the host, the store, a rule).
    Failed(Box<ProjectBoardError>),
}

/// What an operation came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardOutcome {
    Bootstrapped {
        head: BoardHead,
        created: bool,
    },
    /// Tasks listed, each with its summary when one was asked for and
    /// read, or why none was.
    Listing(Listed),
    /// A task with its plans and runs, as one read of the board found it,
    /// and which plan the request asked for, and how much of it.
    Shown(ShownTask, PlanView),
    /// A task changed; after a change that may complete it (and when it
    /// links pull requests), what came of completing it.
    Changed {
        task: Task,
        completion: Option<Completion>,
    },
    Cleared {
        head: BoardHead,
    },
    /// A plan version proposed, edited or discarded, as written (#2481
    /// M4b).
    Planned(Plan),
    /// A draft accepted (#2481 M4c): the task naming it, the plan now
    /// accepted, and the version it superseded, if one was accepted
    /// (boxed: three artifacts).
    Accepted(Box<Accepted>),
    /// A run ended by hand, as written (S4c).
    RunEnded(EndedRun),
    /// A task started and its coordinator launched (S4c).
    Started(Box<StartedTask>),
    /// What a board server answered (epic #2705 B7), rendered there as
    /// this front end renders an answer.
    Served(ServedAnswer),
}

/// A board server's answer (epic #2705 B7): the JSON object its front
/// end rendered, as text, handed on as it came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedAnswer(pub String);

/// A request as its front end read it (epic #2705 B7): typed, and the
/// JSON text that front end read it from. The local adapter runs the
/// typed request; one over a wire sends the text, so the board server
/// reads it again exactly as the front end did.
#[derive(Debug, Clone, Copy)]
pub struct ReadRequest<'a> {
    pub request: &'a BoardRequest,
    pub text: &'a str,
    pub front_end: FrontEnd,
}

/// Whether a board request's run notes what its use cases saw, for the
/// call's record (epic #2481 T4): only a recorded call asks, so an
/// unrecorded one notes and times nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpNoting {
    Noted,
    Unnoted,
}

/// A request the board answered: what it came to, and, when it reads the
/// clock and the clock has no outside reference, why it was not bounded
/// against an outside time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardAnswered {
    pub outcome: BoardOutcome,
    pub clock_note: Option<String>,
}

/// What one board request came to, as a front end is handed it: the
/// answer or the board's refusal, and what its use cases saw, refusals
/// included, when it was noted ([`OpNoting::Noted`]; `None` otherwise).
/// Owned values only, so an adapter may carry it over a wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardExecution {
    pub result: Result<BoardAnswered, ProjectBoardError>,
    pub facts: Option<OpFacts>,
}

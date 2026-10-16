//! The board operations every front end serves (#2477): one typed request
//! ([`BoardRequest`]) that the
//! [`RunBoardOperation`](super::use_cases::RunBoardOperation) use case runs
//! over the board's use cases and listings (composition builds and injects
//! them, [`BoardOperationParts`]) into one typed outcome. The front ends
//! (the `quecto board` CLI, the agent `board` tool) read and answer JSON
//! in the interface layer; here nothing is JSON.
use std::collections::BTreeMap;
use std::sync::Arc;

pub use super::dto::NoPullRequestHost;
use super::dto::{
    BoardOp, DraftUpdate, EndedRun, NewBoardTask, PlanAcceptance, PlanDiscard, PlanEdit,
    PlanProposal, ProjectBoardError, RunEnd, ShownTask, TaskStart,
};
use super::ports::{BoardClock, BoardHead, BoardListings, KeepCopy};
use super::task_summary::Listed;
use super::use_cases::{
    AcceptPlan, BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask,
    CreateBoardTask, DiscardPlan, EditPlan, EndRun, ProposePlan, QueryBoard, StartBoardTask,
    UpdateDraftTask,
};
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::plans::Accepted;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// `BoardListings` over `QueryBoard`, and the operations assembled as
/// composition assembles them, for the front ends' tests.
#[cfg(test)]
#[path = "query_listings_tests.rs"]
pub(crate) mod query_listings;

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
/// round 2): the approach from its character `approach_from`, the task
/// items from the index `items_from`. A part not named is not shown, so
/// a page names at least one ([`Self::of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanPage {
    approach_from: Option<usize>,
    items_from: Option<usize>,
}

impl PlanPage {
    /// The page from `approach_from` and `items_from`; none when neither
    /// is named.
    pub fn of(approach_from: Option<usize>, items_from: Option<usize>) -> Option<Self> {
        match (approach_from, items_from) {
            (None, None) => None,
            (Some(_), _) | (_, Some(_)) => Some(Self {
                approach_from,
                items_from,
            }),
        }
    }

    pub fn approach_from(&self) -> Option<usize> {
        self.approach_from
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
    Started(Box<crate::application::project_board::start::StartedTask>),
}

/// The use cases and ports one run of [`RunBoardOperation`] is made of,
/// built by composition over one board.
///
/// [`RunBoardOperation`]: super::use_cases::RunBoardOperation
pub struct BoardOperationParts {
    pub clock: Arc<dyn BoardClock>,
    pub listings: Arc<dyn BoardListings>,
    /// What the listings (the index) do not hold yet: a task with its
    /// plans and runs, read from the store's snapshot (#2481 M4a; S12 may
    /// answer it from the index).
    pub query: QueryBoard,
    pub bootstrap: BootstrapProjectBoard,
    pub create: CreateBoardTask,
    pub update: UpdateDraftTask,
    pub change: ChangeBoardTask,
    pub propose_plan: ProposePlan,
    pub edit_plan: EditPlan,
    pub accept_plan: AcceptPlan,
    pub discard_plan: DiscardPlan,
    pub end_run: EndRun,
    pub start: StartBoardTask,
    pub clear: ClearSetAside,
    pub complete: Result<CompleteMergedTask, NoPullRequestHost>,
}

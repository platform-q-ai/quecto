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
use super::dto::{BoardListing, BoardOp, DraftUpdate, NewBoardTask, ProjectBoardError};
use super::ports::{BoardClock, BoardHead, BoardListings, KeepCopy};
use super::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    UpdateDraftTask,
};
use crate::domain::project_board::entities::task::Task;
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
    Show(Slug),
    Create(NewBoardTask),
    UpdateDraft(DraftUpdate),
    Change(Slug, BoardOp),
    Complete(Slug),
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
            | Self::Clear { .. } => true,
            Self::List(_) | Self::ListReady | Self::ListTakeable | Self::Show(_) => false,
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
            | Self::ListTakeable => true,
            Self::Bootstrap
            | Self::Clear { .. }
            | Self::List(_)
            | Self::ListReady
            | Self::Show(_) => false,
        }
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
    Listing(BoardListing),
    Shown {
        head: BoardHead,
        task: Task,
    },
    /// A task changed; after a change that may complete it (and when it
    /// links pull requests), what came of completing it.
    Changed {
        task: Task,
        completion: Option<Completion>,
    },
    Cleared {
        head: BoardHead,
    },
}

/// The use cases and ports one run of [`RunBoardOperation`] is made of,
/// built by composition over one board.
///
/// [`RunBoardOperation`]: super::use_cases::RunBoardOperation
pub struct BoardOperationParts {
    pub clock: Arc<dyn BoardClock>,
    pub listings: Arc<dyn BoardListings>,
    pub bootstrap: BootstrapProjectBoard,
    pub create: CreateBoardTask,
    pub update: UpdateDraftTask,
    pub change: ChangeBoardTask,
    pub clear: ClearSetAside,
    pub complete: Result<CompleteMergedTask, NoPullRequestHost>,
}

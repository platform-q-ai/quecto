//! Runs one typed board operation.
//!
//! - After every pull request linked and every move to review, the task
//!   is completed if its pull requests have all merged
//!   (`CompleteMergedTask`), and the outcome says what came of that.
//! - [`BoardOperations::clock_note`] says why a change that stamps tasks
//!   was not bounded against an outside time, when the clock has none.
//!
//! Blocking: the store, the listings, the clock and the host all block, so
//! callers run operations off the async workers.
use std::sync::Arc;

use crate::application::project_board::dto::{
    BoardListing, BoardOp, DraftUpdate, NewBoardTask, ProjectBoardError,
};
use crate::application::project_board::ports::{
    BoardBootstrap, BoardClock, BoardHead, BoardListings, ProjectBoardStore, PullRequestStates,
    PullRequestStatesError, ReferenceStatus,
};
use crate::application::project_board::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    UpdateDraftTask,
};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

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
    Clear { path: String, keep: Option<String> },
}

impl BoardRequest {
    /// The request writes the board (so it needs a committer).
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

    /// The request stamps tasks with the clock's time (so whether that time
    /// was bounded against an outside one matters).
    pub fn stamps(&self) -> bool {
        match self {
            Self::Create(_) | Self::UpdateDraft(_) | Self::Change(..) | Self::Complete(_) => true,
            Self::Bootstrap
            | Self::Clear { .. }
            | Self::List(_)
            | Self::ListReady
            | Self::ListTakeable
            | Self::Show(_) => false,
        }
    }
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
    /// A task changed; after a change that may complete it, what came of
    /// completing it.
    Changed {
        task: Task,
        completion: Option<Box<Result<Task, ProjectBoardError>>>,
    },
    Cleared {
        head: BoardHead,
    },
}

pub struct BoardOperations {
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    bootstrap: BootstrapProjectBoard,
    create: CreateBoardTask,
    update: UpdateDraftTask,
    change: ChangeBoardTask,
    clear: ClearSetAside,
    /// Why not, when the remote's host cannot report pull requests.
    complete: Result<CompleteMergedTask, String>,
}

impl BoardOperations {
    /// The operations over `store`, its `listings` and `clock`, with pull
    /// requests read from `prs` (or none, and why).
    ///
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        listings: Arc<dyn BoardListings>,
        prs: Result<Arc<dyn PullRequestStates>, String>,
    ) -> Result<Self, ProjectBoardError> {
        let complete = match prs {
            Ok(prs) => Ok(CompleteMergedTask::new(
                Arc::clone(&store),
                Arc::clone(&clock),
                prs,
            )?),
            Err(why) => Err(why),
        };
        Ok(Self {
            bootstrap: BootstrapProjectBoard::new(Arc::clone(&store)),
            create: CreateBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?,
            update: UpdateDraftTask::new(Arc::clone(&store), Arc::clone(&clock))?,
            change: ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?,
            clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock))?,
            complete,
            clock,
            listings,
        })
    }

    /// What `request` comes to.
    ///
    /// # Errors
    /// The use case's or the listing's refusal.
    pub fn execute(&self, request: &BoardRequest) -> Result<BoardOutcome, ProjectBoardError> {
        let listing = |listing: Result<BoardListing, _>| {
            listing
                .map(BoardOutcome::Listing)
                .map_err(ProjectBoardError::from)
        };
        match request {
            BoardRequest::Bootstrap => Ok(match self.bootstrap.execute()? {
                BoardBootstrap::Created(head) => BoardOutcome::Bootstrapped {
                    head,
                    created: true,
                },
                BoardBootstrap::Existing(head) => BoardOutcome::Bootstrapped {
                    head,
                    created: false,
                },
            }),
            BoardRequest::List(filter) => listing(match filter {
                ListFilter::All => self.listings.all(),
                ListFilter::Status(status) => self.listings.by_status(*status),
                ListFilter::Children(parent) => self.listings.children(parent),
                ListFilter::Dependents(task) => self.listings.dependents(task),
            }),
            BoardRequest::ListReady => listing(self.listings.ready()),
            BoardRequest::ListTakeable => {
                let now = self.clock.now()?;
                listing(self.listings.takeable(&now))
            }
            BoardRequest::Show(id) => self.show(id),
            BoardRequest::Create(new) => Ok(changed(self.create.execute(new)?, None)),
            BoardRequest::UpdateDraft(update) => Ok(changed(self.update.execute(update)?, None)),
            BoardRequest::Change(id, op) => {
                let task = self.change.execute(id, op)?;
                let completion = completes(op).then(|| Box::new(self.complete(id)));
                Ok(changed(task, completion))
            }
            BoardRequest::Complete(id) => Ok(changed(self.complete(id)?, None)),
            BoardRequest::Clear { path, keep } => Ok(BoardOutcome::Cleared {
                head: self.clear.execute(path, keep.as_deref())?,
            }),
        }
    }

    /// Why a request that stamps tasks was not bounded against an outside
    /// time, when the clock has no reference; `None` for any other request.
    pub fn clock_note(&self, request: &BoardRequest) -> Option<String> {
        match (request.stamps(), self.clock.reference_status()) {
            (true, ReferenceStatus::NoReference(why)) => Some(why),
            (true, ReferenceStatus::Referenced) | (false, _) => None,
        }
    }

    fn complete(&self, id: &Slug) -> Result<Task, ProjectBoardError> {
        match &self.complete {
            Ok(complete) => complete.execute(id),
            Err(why) => Err(ProjectBoardError::PullRequests(
                PullRequestStatesError::Unavailable(why.clone()),
            )),
        }
    }

    fn show(&self, id: &Slug) -> Result<BoardOutcome, ProjectBoardError> {
        let board = self.listings.all()?;
        match board.tasks.iter().find(|task| task.fields().id == *id) {
            Some(task) => Ok(BoardOutcome::Shown {
                head: board.head.clone(),
                task: task.clone(),
            }),
            None if board
                .quarantined
                .iter()
                .any(|file| file.id.as_ref() == Some(id)) =>
            {
                Err(ProjectBoardError::Quarantined(id.clone()))
            }
            None => Err(ProjectBoardError::NotFound(id.clone())),
        }
    }
}

fn changed(task: Task, completion: Option<Box<Result<Task, ProjectBoardError>>>) -> BoardOutcome {
    BoardOutcome::Changed { task, completion }
}

/// A change after which the task may be done: a pull request linked (it
/// may have merged before), or a move to review.
fn completes(op: &BoardOp) -> bool {
    matches!(op, BoardOp::AddPr(_) | BoardOp::Move(TaskStatus::Review))
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

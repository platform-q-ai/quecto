//! What the project board's use cases answer and refuse.
use std::fmt;

use super::ports::{
    ArtifactKey, BoardClockError, BoardHead, BoardSnapshot, BoardStoreError,
    PullRequestStatesError, QuarantinedFile,
};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::artifacts_of;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};

/// A new task: only the task's own fields. Its plan and team are plan
/// artifacts (#2481's artifact model), and its claim, runs, reviews, pull
/// requests and task items are made by the board's later changes, so none can
/// be supplied here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBoardTask {
    pub id: Slug,
    pub title: String,
    pub kind: TaskKind,
    /// Markdown.
    pub description: String,
    pub status: NewBoardTaskStatus,
    pub parent: Option<Slug>,
    pub depends_on: Vec<Slug>,
}

/// An edit of a draft in place: the fields it sets (`None` keeps one as
/// it is; `parent: Some(None)` takes the parent away), and the `updated`
/// the caller read, when it read one: a draft changed since is refused as
/// stale, never edited over. Drafts are shared planning: anyone with
/// access to the board edits any draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftUpdate {
    pub id: Slug,
    pub title: Option<String>,
    /// Markdown.
    pub description: Option<String>,
    pub parent: Option<Option<Slug>>,
    pub depends_on: Option<Vec<Slug>>,
    pub expected_updated: Option<Timestamp>,
}

/// The statuses a task is created in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewBoardTaskStatus {
    Draft,
    Ready,
}

impl From<NewBoardTaskStatus> for TaskStatus {
    fn from(status: NewBoardTaskStatus) -> Self {
        match status {
            NewBoardTaskStatus::Draft => Self::Draft,
            NewBoardTaskStatus::Ready => Self::Ready,
        }
    }
}

/// A change to one task, made by the board's rules
/// (`domain::project_board::services`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardOp {
    /// The holder's claim runs two hours from now again.
    Renew,
    /// Take an expired claim over.
    TakeOver,
    /// Move the task through the transition table; `Move(Claimed)` claims
    /// a ready task.
    Move(TaskStatus),
    /// The holder links a pull request.
    AddPr(u64),
    /// The holder unlinks a pull request deliberately (a wrong or
    /// abandoned one); a task whose pull requests all merged is closed by
    /// `CompleteMergedTask`, and once none is linked, Review→Done is the
    /// holder's.
    RemovePr(u64),
}

impl BoardOp {
    /// The board commit message of this op on task `id`.
    pub fn message(&self, id: &Slug) -> String {
        let id = id.as_str();
        match self {
            Self::Renew => format!("board: renew {id}"),
            Self::TakeOver => format!("board: take over {id}"),
            Self::Move(to) => format!("board: move {id} to {}", to.as_str()),
            Self::AddPr(pr) => format!("board: link pull request {pr} to {id}"),
            Self::RemovePr(pr) => format!("board: unlink pull request {pr} from {id}"),
        }
    }
}

/// Tasks a query found, by id, and every file the board set aside, so a
/// listing never hides that part of the board could not be read; both as
/// the board stood at `head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardListing {
    /// The head the listing was read at.
    pub head: BoardHead,
    pub tasks: Vec<Task>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// One kind of artifact a query found, in the query's order, and every
/// file the board set aside (of any kind, a stray included), so a listing
/// never hides that part of the board could not be read; both as the
/// board stood at `head`. An item may point at what the board does not
/// hold (a plan of a task that is gone): each file is judged on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactListing<T> {
    /// The head the listing was read at.
    pub head: BoardHead,
    /// Which artifacts it lists.
    pub scope: ArtifactScope,
    pub items: Vec<T>,
    pub quarantined: Vec<QuarantinedFile>,
}

/// Which artifacts a listing of a task's other artifacts lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactScope {
    Plans {
        task: Slug,
    },
    /// Every run of the task, whichever plan version it ran.
    Runs {
        task: Slug,
    },
    RunsOfPlan {
        task: Slug,
        version: u32,
    },
    Changes {
        task: Slug,
    },
    Review {
        task: Slug,
        change: u32,
    },
    Discussions {
        task: Slug,
    },
}

impl ArtifactScope {
    /// Whether a file set aside under `key` may hold one of the listed
    /// artifacts. A run's key names no plan version, and a discussion's
    /// no task, so any of the task's runs, or any discussion, may.
    pub fn covers(&self, key: &ArtifactKey) -> bool {
        match (self, key) {
            (Self::Plans { task }, ArtifactKey::Plan(of, _))
            | (Self::Runs { task }, ArtifactKey::Run(of, _))
            | (Self::RunsOfPlan { task, .. }, ArtifactKey::Run(of, _))
            | (Self::Changes { task }, ArtifactKey::Change(of, _)) => of == task,
            (Self::Review { task, change }, ArtifactKey::Review(of, number)) => {
                of == task && number == change
            }
            (Self::Discussions { .. }, ArtifactKey::Discussion(_)) => true,
            (
                Self::Plans { .. }
                | Self::Runs { .. }
                | Self::RunsOfPlan { .. }
                | Self::Changes { .. }
                | Self::Review { .. }
                | Self::Discussions { .. },
                _,
            ) => false,
        }
    }

    /// Whether the set-aside `file` may hold one of the listed artifacts:
    /// one whose name spells a key this scope [covers](Self::covers); or,
    /// whose name spells none, one that is kept in the listed kind's own
    /// folder (it may hold any of that kind), never one in another kind's
    /// folder (a task's README holds no plan), nor a stray (it holds
    /// nothing that stays).
    pub fn holds(&self, file: &QuarantinedFile) -> bool {
        match &file.key {
            Some(key) => self.covers(key),
            None => {
                let in_folder = file
                    .path
                    .strip_prefix(self.folder())
                    .is_some_and(|rest| rest.starts_with('/'));
                file.carried && in_folder
            }
        }
    }

    /// The board folder the listed kind's files lie in.
    pub fn folder(&self) -> &'static str {
        match self {
            Self::Plans { .. } => "plans",
            Self::Runs { .. } | Self::RunsOfPlan { .. } => "runs",
            Self::Changes { .. } => "changes",
            Self::Review { .. } => "reviews",
            Self::Discussions { .. } => "discussions",
        }
    }
}

impl<T> ArtifactListing<T> {
    /// The files set aside that may hold one of the listed artifacts
    /// ([`ArtifactScope::holds`]): while any is, the listing may be
    /// incomplete for its scope. A kept file set aside whose name spells
    /// no key may hold anything of its folder's kind, so it counts in
    /// that kind's scopes; a stray (not kept) holds nothing that stays, so
    /// it counts in none.
    pub fn set_aside_in_scope(&self) -> Vec<&QuarantinedFile> {
        self.quarantined
            .iter()
            .filter(|file| self.scope.holds(file))
            .collect()
    }
}

impl<T: Clone> ArtifactListing<T> {
    /// `items`, picked from `board` for `scope`, beside its head and
    /// every file it set aside; refused if one key is there twice.
    fn picked(
        board: &BoardSnapshot,
        scope: ArtifactScope,
        items: Vec<&T>,
        key: impl Fn(&T) -> ArtifactKey,
    ) -> Result<Self, ProjectBoardError> {
        Ok(Self {
            head: board.head.clone(),
            scope,
            items: once(items, key)?.into_iter().cloned().collect(),
            quarantined: board.quarantined.clone(),
        })
    }
}

/// `items` (ordered by `key`), unless one key is there more than once: a
/// store broke its contract of one artifact per key, so what the board
/// holds is unknown. Every listing and board view asks here.
pub(crate) fn once<T>(
    items: Vec<&T>,
    key: impl Fn(&T) -> ArtifactKey,
) -> Result<Vec<&T>, ProjectBoardError> {
    let keys: Vec<ArtifactKey> = items.iter().map(|item| key(item)).collect();
    match keys.windows(2).find(|pair| pair[0] == pair[1]) {
        Some(pair) => Err(ProjectBoardError::Duplicate {
            key: pair[0].clone(),
        }),
        None => Ok(items),
    }
}

impl ArtifactListing<Plan> {
    /// Every plan of `task` on `board`, by version.
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a plan the board holds twice.
    pub fn plans_of(board: &BoardSnapshot, task: &Slug) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::Plans { task: task.clone() };
        let plans = artifacts_of::plans_of(&board.plans, task);
        Self::picked(board, scope, plans, ArtifactKey::of_plan)
    }
}

impl ArtifactListing<Run> {
    /// Every run of `task` on `board`, by number, whichever plan version
    /// it ran (that plan on the board or not).
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a run the board holds twice.
    pub fn runs_of(board: &BoardSnapshot, task: &Slug) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::Runs { task: task.clone() };
        let runs = artifacts_of::runs_of(&board.runs, task);
        Self::picked(board, scope, runs, ArtifactKey::of_run)
    }

    /// Every run of `task` of its plan's `version` on `board`, by number.
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a run the board holds twice.
    pub fn runs_of_plan(
        board: &BoardSnapshot,
        task: &Slug,
        version: u32,
    ) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::RunsOfPlan {
            task: task.clone(),
            version,
        };
        // Each run once by its key first: a copy of another plan version
        // is still a duplicate.
        let runs = once(
            artifacts_of::runs_of(&board.runs, task),
            ArtifactKey::of_run,
        )?;
        let runs = artifacts_of::of_plan_version(runs, version);
        Self::picked(board, scope, runs, ArtifactKey::of_run)
    }
}

impl ArtifactListing<Change> {
    /// Every change of `task` on `board`, by number.
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a change the board holds twice.
    pub fn changes_of(board: &BoardSnapshot, task: &Slug) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::Changes { task: task.clone() };
        let changes = artifacts_of::changes_of(&board.changes, task);
        Self::picked(board, scope, changes, ArtifactKey::of_change)
    }
}

impl ArtifactListing<Review> {
    /// The review of `task`'s change `change` on `board`: none or one.
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a review the board holds twice.
    pub fn review_of(
        board: &BoardSnapshot,
        task: &Slug,
        change: u32,
    ) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::Review {
            task: task.clone(),
            change,
        };
        let reviews = artifacts_of::reviews_of_change(&board.reviews, task, change);
        Self::picked(board, scope, reviews, ArtifactKey::of_review)
    }
}

impl ArtifactListing<Discussion> {
    /// Every discussion linked to `task` on `board`, by id.
    ///
    /// # Errors
    /// [`ProjectBoardError::Duplicate`] for a discussion the board holds
    /// twice.
    pub fn discussions_of(board: &BoardSnapshot, task: &Slug) -> Result<Self, ProjectBoardError> {
        let scope = ArtifactScope::Discussions { task: task.clone() };
        // Each discussion once by its key first: a copy linked to another
        // task is still a duplicate.
        let mut every: Vec<&Discussion> = board.discussions.iter().collect();
        every.sort_by(|a, b| a.fields().id.cmp(&b.fields().id));
        once(every, ArtifactKey::of_discussion)?;
        let discussions = artifacts_of::discussions_of(&board.discussions, task);
        Self::picked(board, scope, discussions, ArtifactKey::of_discussion)
    }
}

/// A task as `show` gives it (#2481 M4a): the task, every version of its
/// plan by version and every run of it by number, and the files set aside
/// that may hold one of its plans or runs, all as one snapshot of the
/// board stood at `head`. While any file is set aside in their scope, the
/// plans and runs may be incomplete: a set-aside file is named, never a
/// refusal, since the task itself was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownTask {
    pub head: BoardHead,
    pub task: Task,
    pub plans: Vec<Plan>,
    pub runs: Vec<Run>,
    pub set_aside: Vec<QuarantinedFile>,
}

impl ShownTask {
    /// The task's plan: the version the task names as accepted, when the
    /// board holds it accepted. Where the two disagree (a board whose
    /// files contradict each other), or the plan is set aside, none is.
    pub fn accepted_plan(&self) -> Option<&Plan> {
        let version = self.task.fields().accepted_plan?;
        self.plans.iter().find(|plan| {
            let plan = plan.fields();
            plan.version == version && plan.state == PlanState::Accepted
        })
    }

    /// The plan a show gives: version `version` when one is named, the
    /// accepted plan (if any) otherwise.
    ///
    /// # Panics
    /// A version named that [`Self::version`] refuses: a show is answered
    /// only once `RunBoardOperation` found the version it names.
    pub fn plan(&self, version: Option<u32>) -> Option<&Plan> {
        match version {
            Some(version) => match self.version(version) {
                Ok(plan) => Some(plan),
                Err(error) => panic!("a show names only a version it found, not: {error}"),
            },
            None => self.accepted_plan(),
        }
    }

    /// Version `version` of the task's plan, whatever its state (#2481
    /// M4b: a draft is read before it is edited or accepted).
    ///
    /// # Errors
    /// [`ProjectBoardError::Quarantined`] while its own file is set aside
    /// (checked first: what the board holds of it is then unknown);
    /// [`ProjectBoardError::SetAsideFile`] when the task has none read
    /// while a kept file in `plans/` whose name spells no key is set aside
    /// (it may hold that version); otherwise the rule
    /// [`BoardRuleError::UnknownPlanVersion`] when the task has none.
    pub fn version(&self, version: u32) -> Result<&Plan, ProjectBoardError> {
        let id = &self.task.fields().id;
        let key = ArtifactKey::Plan(id.clone(), version);
        if let Some(file) = self
            .set_aside
            .iter()
            .find(|file| file.key.as_ref() == Some(&key))
        {
            return Err(ProjectBoardError::Quarantined {
                key,
                path: file.path.clone(),
            });
        }
        if let Some(plan) = self
            .plans
            .iter()
            .find(|plan| plan.fields().version == version)
        {
            return Ok(plan);
        }
        let plans = ArtifactScope::Plans { task: id.clone() };
        let keyless = self
            .set_aside
            .iter()
            .filter(|file| file.key.is_none() && plans.holds(file))
            .min_by(|a, b| (&a.path, &a.object).cmp(&(&b.path, &b.object)));
        match keyless {
            Some(file) => Err(ProjectBoardError::SetAsideFile {
                path: file.path.clone(),
                object: file.object.clone(),
            }),
            None => Err(ProjectBoardError::Rule(BoardRuleError::UnknownPlanVersion(
                version,
            ))),
        }
    }

    /// The task's latest run: the one numbered last.
    pub fn latest_run(&self) -> Option<&Run> {
        self.runs.last()
    }

    /// Whether a file that may hold one of its plans or runs is set aside.
    pub fn may_be_incomplete(&self) -> bool {
        !self.set_aside.is_empty()
    }
}

/// An artifact besides a task that uses a task's id: a plan, run, change
/// or review of the task by that id (by its number), or a discussion
/// about it (by its own id). A task cannot be one, so an id in use is
/// never said to be used by itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum IdUser {
    Plan(u32),
    Run(u32),
    Change(u32),
    Review(u32),
    Discussion(Slug),
}

impl IdUser {
    /// Its key on the board, for the task by the id `id`.
    pub fn key(&self, id: &Slug) -> ArtifactKey {
        match self {
            Self::Plan(version) => ArtifactKey::Plan(id.clone(), *version),
            Self::Run(number) => ArtifactKey::Run(id.clone(), *number),
            Self::Change(number) => ArtifactKey::Change(id.clone(), *number),
            Self::Review(change) => ArtifactKey::Review(id.clone(), *change),
            Self::Discussion(discussion) => ArtifactKey::Discussion(discussion.clone()),
        }
    }
}

/// Why a board operation did nothing.
/// Why the board's remote reports no pull requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoPullRequestHost {
    /// The remote is on no host pull requests are read from.
    NotAPullRequestHost,
    /// The host is one, but cannot be asked (no `gh`, say): why.
    Unreachable(String),
}

impl fmt::Display for NoPullRequestHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPullRequestHost => f.write_str(
                "the board remote is not on GitHub or the configured host, so no pull request \
                 states are read",
            ),
            Self::Unreachable(why) => write!(f, "pull request states cannot be read: {why}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectBoardError {
    Store(BoardStoreError),
    /// The board's rules refused the change, read against the latest task.
    Rule(BoardRuleError),
    /// The task would break the schema.
    Invalid(SchemaError),
    NotFound(Slug),
    Exists(Slug),
    /// No task `id` is on the board, but the artifact `by` (a plan, run,
    /// change or review of a task by that id, or a discussion about it:
    /// never a task, which [`IdUser`] cannot hold) already names it: a new
    /// task would take over what it says. While a file that may name it is
    /// set aside (one of that task's, or any discussion) a create is
    /// refused as set aside instead: creates are rare, so it waits for the
    /// mend.
    IdInUse {
        id: Slug,
        by: IdUser,
    },
    /// This machine's clock could not tell the time.
    Clock(BoardClockError),
    /// The artifact's file (at `path`) is set aside as unreadable: mend
    /// or clear it first.
    Quarantined {
        key: ArtifactKey,
        path: String,
    },
    /// The board holds the artifact `key` more than once: its store broke
    /// the contract of one artifact per key, so what it holds is unknown.
    Duplicate {
        key: ArtifactKey,
    },
    /// A file every write carries is set aside whose task cannot be told
    /// (no id could be read from it): what it holds is unknown until it is
    /// mended or cleared (by `path` and `object`).
    SetAsideFile {
        path: String,
        object: String,
    },
    /// A new task names a parent or dependency the board does not hold.
    UnknownReference(Slug),
    /// Creating task `id` would close a cycle: its parent and
    /// dependencies lead, through the board, to task `by`, which already
    /// names `id` as its parent or a dependency (`id`'s own task was
    /// deleted).
    ClosesCycle {
        id: Slug,
        by: Slug,
    },
    /// The store writes as an identity no task can name.
    InvalidWriter(SchemaError),
    /// The host does not know a pull request the task links.
    PrStateUnknown(u64),
    /// The host could not tell where the task's pull requests stand.
    PullRequests(PullRequestStatesError),
    /// The task was stamped past the reference by a clock that ran ahead:
    /// it is not built on until real time catches up.
    TaskAhead {
        id: Slug,
        updated: Timestamp,
    },
    /// The outside reference reads a time the board cannot spell.
    ClockOutOfRange(Timestamp),
    /// This machine's clock is more than [`CLOCK_AHEAD_SECONDS`] ahead of
    /// the outside reference.
    ///
    /// [`CLOCK_AHEAD_SECONDS`]: crate::domain::project_board::services::claims::CLOCK_AHEAD_SECONDS
    ClockAhead {
        now: Timestamp,
        reference: Timestamp,
    },
    /// Every attempt lost the race to another writer.
    Contended {
        attempts: usize,
    },
    /// The store refused the write.
    Refused(String),
    /// No file is set aside at this path.
    NotSetAside {
        path: String,
        object: String,
    },
    /// The board's remote reports no pull requests, so none is completed.
    NoPullRequestHost(NoPullRequestHost),
    /// Task `id` changed since the caller read it (`expected` its
    /// `updated` then, `updated` now): read it again and decide again.
    Stale {
        id: Slug,
        expected: Timestamp,
        updated: Timestamp,
    },
}

impl From<BoardStoreError> for ProjectBoardError {
    fn from(error: BoardStoreError) -> Self {
        Self::Store(error)
    }
}

/// A rule's refusal, as the board's rules refuse a change.
impl From<BoardRuleError> for ProjectBoardError {
    fn from(error: BoardRuleError) -> Self {
        Self::Rule(error)
    }
}

impl From<BoardClockError> for ProjectBoardError {
    fn from(error: BoardClockError) -> Self {
        Self::Clock(error)
    }
}

impl fmt::Display for ProjectBoardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(BoardStoreError::NoBoard) => f.write_str("the project has no board yet"),
            Self::Store(BoardStoreError::Clash(why)) => {
                write!(f, "the board clashes with the repository: {why}")
            }
            Self::Store(BoardStoreError::InvalidWrite(why)) => {
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
        }
    }
}

impl std::error::Error for ProjectBoardError {}

#[path = "dto_plans.rs"]
mod plans;
pub use plans::{PlanEdit, PlanProposal};

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;

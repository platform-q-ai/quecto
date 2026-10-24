//! The project board capability's ports (#2476): where the board lives
//! (the S1d git store implements [`ProjectBoardStore`] over the
//! repository's `quecto/board` branch), its listings from a local view of
//! it (the S1e index implements [`BoardListings`]) and what time it is.
use super::dto::{
    BoardExecution, BoardListing, BoardRequest, FoldBackFacts, FoldedBack, ListSummaries, OpNoting,
    ProjectBoardError, RunEnd, SwarmRunView,
};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::board_moved::BoardMoved;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::{
    BoardOpRecord, BoardStartRecord, FoldBackRecord,
};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{Effort, TaskStatus};

mod served;
pub use served::{ServeError, ServedStatus, ServerRole};
mod listen_address;
pub use listen_address::ListenAddress;
mod tokens;
pub use tokens::{BoardTokens, IssuedToken, MAX_TOKENS, TokenError, TokenRecord};

/// The board's head: the store's revision a write must start from (the
/// git store's commit id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoardHead(pub String);

/// Which artifact a board file holds (#2481's artifact model): what names
/// it on the board, one file each. A task by its id; a plan, run or change
/// by its task and number; a review by its task and its change's number; a
/// discussion by its id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactKey {
    Task(Slug),
    Plan(Slug, u32),
    Run(Slug, u32),
    Change(Slug, u32),
    Review(Slug, u32),
    Discussion(Slug),
}

impl ArtifactKey {
    pub fn of_task(task: &Task) -> Self {
        Self::Task(task.fields().id.clone())
    }

    pub fn of_plan(plan: &Plan) -> Self {
        Self::Plan(plan.fields().task.clone(), plan.fields().version)
    }

    pub fn of_run(run: &Run) -> Self {
        Self::Run(run.fields().task.clone(), run.fields().number)
    }

    pub fn of_change(change: &Change) -> Self {
        Self::Change(change.fields().task.clone(), change.fields().number)
    }

    pub fn of_review(review: &Review) -> Self {
        Self::Review(review.fields().task.clone(), review.fields().change)
    }

    pub fn of_discussion(discussion: &Discussion) -> Self {
        Self::Discussion(discussion.fields().id.clone())
    }

    /// The task's id, when it is a task's own key (not the task a plan,
    /// run, change or review belongs to).
    pub fn as_task(&self) -> Option<&Slug> {
        match self {
            Self::Task(id) => Some(id),
            Self::Plan(..)
            | Self::Run(..)
            | Self::Change(..)
            | Self::Review(..)
            | Self::Discussion(_) => None,
        }
    }
}

/// How a message names it: `task x`, `plan 2 of task x`, `review 3 of
/// task x` (the review of its change 3), `discussion d`.
impl std::fmt::Display for ArtifactKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Task(id) => write!(f, "task {}", id.as_str()),
            Self::Plan(id, version) => write!(f, "plan {version} of task {}", id.as_str()),
            Self::Run(id, number) => write!(f, "run {number} of task {}", id.as_str()),
            Self::Change(id, number) => write!(f, "change {number} of task {}", id.as_str()),
            Self::Review(id, change) => write!(f, "review {change} of task {}", id.as_str()),
            Self::Discussion(id) => write!(f, "discussion {}", id.as_str()),
        }
    }
}

/// One artifact as a write gives it, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardArtifact {
    Task(Task),
    Plan(Plan),
    Run(Run),
    Change(Change),
    Review(Review),
    Discussion(Discussion),
}

impl BoardArtifact {
    /// The key its file is named by.
    pub fn key(&self) -> ArtifactKey {
        match self {
            Self::Task(task) => ArtifactKey::of_task(task),
            Self::Plan(plan) => ArtifactKey::of_plan(plan),
            Self::Run(run) => ArtifactKey::of_run(run),
            Self::Change(change) => ArtifactKey::of_change(change),
            Self::Review(review) => ArtifactKey::of_review(review),
            Self::Discussion(discussion) => ArtifactKey::of_discussion(discussion),
        }
    }
}

/// A board file the store set aside, and why. `key` is the artifact its
/// file name spells, when it spells one: none may be written under that key
/// until the file is mended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedFile {
    pub path: String,
    pub key: Option<ArtifactKey>,
    pub reason: String,
    /// The store's name for the file's contents (an object id): with
    /// `path`, which file this is, since two files may show the same path.
    pub object: String,
    /// Every write carries it forward as it is (a task file the store
    /// could not read): what it holds stays on the board. A file not
    /// carried (a stray) is dropped by the next write. The one exception:
    /// the copies of a name listed more than once are not carried, yet not
    /// dropped either: every write that would rebuild their folder (every
    /// write, in `tasks/`) is refused until a clear resolves them (it names
    /// the copy kept, or that none is).
    pub carried: bool,
}

impl QuarantinedFile {
    /// The task id its name spells, when it names a task file.
    pub fn id(&self) -> Option<&Slug> {
        self.key.as_ref().and_then(ArtifactKey::as_task)
    }
}

/// What a clear does with a name listed more than once. Each names every
/// copy's object the caller saw (`seen`), so a copy it never saw is never
/// dropped: a name whose copies are not exactly those is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepCopy {
    /// The copy whose object is `keep` stays as it was; every other copy,
    /// each of `seen`, goes. `keep` is one of `seen`.
    Object {
        keep: String,
        seen: std::collections::BTreeSet<String>,
    },
    /// Every copy goes: exactly the copies whose objects these are.
    Drop(std::collections::BTreeSet<String>),
}

/// A clear of files the board set aside, naming each as the caller saw it
/// in the board's listing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetAsideClear {
    /// The [`QuarantinedFile::path`] of the file to clear.
    pub path: String,
    /// Its [`QuarantinedFile::object`]: a file at that path with other
    /// contents is never cleared in its place.
    pub object: String,
    /// For every name the board lists more than once, by path, which copy
    /// stays: one clear resolves them all, since a board that lists a name
    /// twice is one no remote that checks its objects takes.
    pub repeated: std::collections::BTreeMap<String, KeepCopy>,
}

/// The board at its head: every readable artifact of each kind, and what
/// was set aside. The order of each list is the store's own; read them as
/// sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub head: BoardHead,
    pub tasks: Vec<Task>,
    pub plans: Vec<Plan>,
    pub runs: Vec<Run>,
    pub changes: Vec<Change>,
    pub reviews: Vec<Review>,
    pub discussions: Vec<Discussion>,
    pub quarantined: Vec<QuarantinedFile>,
}

impl BoardSnapshot {
    /// The snapshot, as a store hands it over: debug builds check the
    /// stores' contract, that no task both reads and has its file set
    /// aside (a store sets a task's file aside instead of reading it,
    /// never as well), and that each artifact is held once (no two read
    /// under one key).
    #[must_use]
    pub fn asserted(self) -> Self {
        debug_assert!(
            self.quarantined
                .iter()
                .filter_map(QuarantinedFile::id)
                .all(|id| self.tasks.iter().all(|task| task.fields().id != *id)),
            "a task is set aside and read at once"
        );
        debug_assert!(self.held_once(), "the board holds an artifact twice");
        self
    }

    /// Whether every artifact read is held under its own key once.
    fn held_once(&self) -> bool {
        let keys: Vec<ArtifactKey> = self
            .tasks
            .iter()
            .map(ArtifactKey::of_task)
            .chain(self.plans.iter().map(ArtifactKey::of_plan))
            .chain(self.runs.iter().map(ArtifactKey::of_run))
            .chain(self.changes.iter().map(ArtifactKey::of_change))
            .chain(self.reviews.iter().map(ArtifactKey::of_review))
            .chain(self.discussions.iter().map(ArtifactKey::of_discussion))
            .collect();
        let distinct: std::collections::BTreeSet<&ArtifactKey> = keys.iter().collect();
        distinct.len() == keys.len()
    }
}

/// What became of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardWrite {
    /// The board moved to this head, and it holds the write.
    Written(BoardHead),
    /// The board does NOT hold this write: someone else moved it first.
    /// Read it again. A store that cannot tell whether its write landed
    /// answers neither this nor [`Self::Written`] but an error.
    Lost,
    /// The store refused the write, and the board did not move.
    Refused(String),
}

/// What [`ProjectBoardStore::bootstrap`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardBootstrap {
    Created(BoardHead),
    Existing(BoardHead),
}

/// Why the store could not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardStoreError {
    /// No board yet: bootstrap it.
    NoBoard,
    /// Something in the repository keeps the board from existing.
    Clash(String),
    /// A write no store takes: no artifacts, one key twice, a kind the
    /// store does not keep yet, a value its codec refuses, a message that is
    /// not one short line, a board past its bounds of entries (its bytes
    /// are [`Self::WriteTooLarge`]), or a head that is not
    /// on the board. That includes a head the board held before its
    /// history was rewritten (a force push): such a write is invalid, not
    /// [`BoardWrite::Lost`], since no store can tell it from a head it
    /// never held. Read the board again. Nothing was written.
    InvalidWrite(String),
    /// A write that would leave its files' total bytes, on the board or in
    /// one of its folders, past what a read takes. Nothing was written; a
    /// write of smaller files may land. Front ends answer it as an invalid
    /// write.
    WriteTooLarge(String),
    /// The store could not be reached or read, or cannot tell whether a
    /// write landed.
    Unavailable(String),
    /// The board is past what a read takes: too many entries, or too many
    /// bytes, in a folder or in all. Nothing is read.
    TooLarge(String),
    /// A file set aside outside `tasks/` holds the board where an index
    /// cannot yet judge it: no listing is given while one names an artifact
    /// other than a task (an index that does not yet hold that kind).
    FolderSetAside {
        why: String,
        /// The file set aside, by its path and object, as `clear_set_aside`
        /// names it (#2659).
        path: String,
        object: String,
    },
    /// The board moved to a board server (format 4, #2705 B13): this store
    /// neither reads nor writes it; the server's URLs serve it.
    Moved(BoardMoved),
    /// A board fenced at format 4 whose `MOVED` file is not one quecto
    /// writes: why. Neither read nor written.
    MovedMalformed(String),
}

/// Where a project's board lives. Blocking: callers run it off the async
/// workers.
pub trait ProjectBoardStore: Send + Sync {
    /// The board as it stands now.
    ///
    /// # Errors
    /// No board, or the store's failure.
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError>;

    /// Writes `artifacts` (each whole, replacing any of the same key) in
    /// one change, only if the board is still at `expected`. An artifact
    /// whose key a file set aside holds is [`BoardWrite::Refused`].
    ///
    /// # Errors
    /// An invalid write (no artifacts, a key twice, a kind the store does
    /// not keep yet), or the store's failure; losing the race is
    /// [`BoardWrite::Lost`].
    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError>;

    /// [`Self::write`] of tasks alone.
    ///
    /// # Errors
    /// As [`Self::write`].
    fn write_tasks(
        &self,
        expected: &BoardHead,
        tasks: &[Task],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let artifacts: Vec<BoardArtifact> =
            tasks.iter().cloned().map(BoardArtifact::Task).collect();
        self.write(expected, &artifacts, message)
    }

    /// How many times this store found its board's tracking ref locked by
    /// another writer in the same clone and fetched again (#2654): a count
    /// that only grows, for a write's record (#2663). A store that shares
    /// no ref with other writers counts none.
    fn lock_races(&self) -> u64 {
        0
    }

    /// Removes the file set aside that `clear` names (a
    /// [`QuarantinedFile`] of the board at `expected`, by path and, when
    /// two share a path, object) in one change, made by [`Self::writer`]
    /// so the board records who did, only if the board is still at
    /// `expected`. Any member may clear: the trust model is cooperative
    /// (anyone who can push could push raw commits), and branch protection
    /// or a server hook is the real control.
    ///
    /// While the board lists any name more than once, a clear resolves
    /// every such name ([`SetAsideClear::repeated`]): the copy to keep, or
    /// none. A file that is not set aside, a path two files share with no
    /// object to tell them apart, a name listed more than once left
    /// unresolved, or a copy to keep that is not one of a name's copies is
    /// [`BoardWrite::Refused`]. A clear from a head the board moved past is
    /// [`BoardWrite::Lost`], before anything else is judged.
    ///
    /// # Errors
    /// A message that is not one short line, an object that is not an
    /// object id, a board past its bounds, or the store's failure.
    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError>;

    /// Creates the empty board, unless it exists.
    ///
    /// # Errors
    /// A clash, or the store's failure.
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError>;

    /// The committer identity the store writes as: every board change is
    /// made by it. It cannot fail: a store resolves its identity, and
    /// checks it is a valid one ([`Identity::validate`]), when it is built,
    /// and refuses to be built without one.
    fn writer(&self) -> Identity;
}

/// The board's listings, answered from a local view kept in step with the
/// store: each means what [`QueryBoard`]'s listing of the same name means,
/// and carries the head it was read at and every file set aside.
/// Blocking: callers run it off the async workers.
///
/// Listings are advice. The local view lives in the repository (the
/// index under `.git/quecto`), so anyone who can write there, an agent
/// included, can forge what it lists. Act on a listing only through the
/// store: every write is a compare-and-swap against the board's real head,
/// so a forged listing can never move the board.
///
/// [`QueryBoard`]: super::use_cases::QueryBoard
pub trait BoardListings: Send + Sync {
    /// Ready tasks whose every dependency is on the board and releases its
    /// dependents.
    ///
    /// # Errors
    /// No board, or the view's or store's failure.
    fn ready(&self) -> Result<BoardListing, BoardStoreError>;

    /// Held tasks whose claim expired by `now`.
    ///
    /// # Errors
    /// No board, or the view's or store's failure.
    fn takeable(&self, now: &Timestamp) -> Result<BoardListing, BoardStoreError>;

    /// Every task on the board, from one refresh of the view.
    ///
    /// # Errors
    /// No board, or the view's or store's failure.
    fn all(&self) -> Result<BoardListing, BoardStoreError>;

    /// # Errors
    /// No board, or the view's or store's failure.
    fn by_status(&self, status: TaskStatus) -> Result<BoardListing, BoardStoreError>;

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// No board, or the view's or store's failure.
    fn children(&self, parent: &Slug) -> Result<BoardListing, BoardStoreError>;

    /// The tasks that depend on `task`.
    ///
    /// # Errors
    /// No board, or the view's or store's failure.
    fn dependents(&self, task: &Slug) -> Result<BoardListing, BoardStoreError>;
}

/// Why the clock could not tell the time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardClockError {
    /// This machine's clock reads a time the board cannot spell (before
    /// 1970, or past 9999).
    OutOfRange(String),
}

/// Whether the board clock has an outside reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceStatus {
    Referenced,
    /// No outside time is read, and why: changes are not bounded against
    /// one.
    NoReference(String),
}

/// What time it is, an outside reading of it to check that against, and
/// the pause between attempts at a write. Blocking: [`Self::pause`] holds
/// the calling thread, so callers run board changes off the async workers,
/// as they do the store's.
pub trait BoardClock: Send + Sync {
    /// This machine's time.
    ///
    /// # Errors
    /// A time the board cannot spell.
    fn now(&self) -> Result<Timestamp, BoardClockError>;

    /// The time a source outside this machine reports, when one can be
    /// read; `None` when none can. It may read the network (within a
    /// bound), so it blocks the calling thread.
    fn reference(&self) -> Option<Timestamp>;

    /// Whether [`Self::reference`] reads an outside time, or why it does
    /// not, so a front end can tell its user that changes are not bounded
    /// against one. It reads as `reference` does.
    fn reference_status(&self) -> ReferenceStatus;

    /// Waits before attempt `attempt` (from 2) at a write that lost the
    /// race: a short, jittered pause, so writers that collided spread out.
    /// It blocks the calling thread for the pause.
    fn pause(&self, attempt: usize);

    /// The time since this clock was made, on a monotonic clock: it never
    /// runs back and does not follow this machine's wall time. Time
    /// budgets are measured with it.
    fn elapsed(&self) -> std::time::Duration;
}

/// Where a pull request stands, as the repository's host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRequestState {
    Open,
    Merged,
    /// Closed without merging.
    Closed,
    /// The host does not know this pull request.
    Unknown,
}

/// Why the host could not tell where a pull request stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestStatesError {
    Unavailable(String),
}

/// The repository host's reading of the pull requests a task links.
/// Blocking: callers run it off the async workers.
pub trait PullRequestStates: Send + Sync {
    /// Where pull request `pr` stands now.
    ///
    /// # Errors
    /// The host could not be reached or read.
    fn state(&self, pr: u64) -> Result<PullRequestState, PullRequestStatesError>;
}

/// Port: whether the board's file format holds a line of text as a
/// table's text cell, exactly as the store's codec judges the cells it
/// writes (a run's owners and evidence among them). Text from outside the
/// board (a swarm's evidence) is checked here line by line, so one line
/// the codec would refuse is left out rather than sink the whole write.
pub trait BoardTextCells: Send + Sync {
    /// Whether a text cell holds `line` as it is.
    fn holds(&self, line: &str) -> bool;

    /// Whether a prose section holds `text` (a description's goal or why,
    /// #2705 T2) as it is: written and read back to the same text.
    fn holds_prose(&self, text: &str) -> bool;
}

/// What one fold-back came to, as its recorder is given it.
#[derive(Debug, Clone, Copy)]
pub struct FoldBackOutcome<'a> {
    pub task: &'a Slug,
    pub run: u32,
    pub result: &'a Result<FoldedBack, ProjectBoardError>,
    pub facts: &'a FoldBackFacts,
    /// The whole fold-back, on the board clock's monotonic time.
    pub elapsed: std::time::Duration,
}

/// Port: where a fold-back's outcome goes, its `board_fold_back` record
/// (S4), for the caller in this layer that folds a run back (the keeper).
/// The front end implements it, naming its error and rule codes. A caller
/// holds one only while the event log is on, so with it off no fact is
/// gathered and nothing is timed.
pub trait FoldBackRecorder: Send + Sync {
    /// Records `outcome`, never failing the fold-back.
    fn record(&self, outcome: FoldBackOutcome<'_>);
}

/// Port: where each board request's `board_op` record goes (epic #2481):
/// the event log (a session's, or the CLI's daily `board-cli_<date>`
/// log) and `tracing`. It is held only inside a
/// [`BoardRecording`](super::telemetry::BoardRecording), which exists only
/// while `telemetry.event_log.enabled` is on, so with it off nothing is
/// recorded.
pub trait ProjectBoardOpLog: Send + Sync {
    /// Appends `record` synchronously, from the request's own blocking
    /// thread: never on an async runtime, never panicking. A failed or
    /// dropped write is the adapter's to count and report, and never
    /// changes the request's answer.
    fn record(&self, record: BoardOpRecord);

    /// Appends a fold-back's `board_fold_back` record (S4), as
    /// [`Self::record`] appends a `board_op`.
    fn record_fold_back(&self, record: FoldBackRecord);

    /// Appends a start's `board_start` record (S4c), as [`Self::record`]
    /// appends a `board_op`.
    fn record_start(&self, record: BoardStartRecord);
}

/// What a started task's coordinator is launched with (S4c): the run it
/// coordinates, the brief it is given as its task text, rendered, and the
/// model and effort the plan's coordinator role names (none: the
/// session's own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorRequest {
    pub task: Slug,
    pub run: u32,
    pub plan_version: u32,
    pub brief: String,
    pub model: Option<String>,
    pub effort: Option<Effort>,
}

/// A coordinator launched: its agent's id and the container it runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchedCoordinator {
    pub agent: String,
    pub environment_ref: String,
}

/// Why a coordinator did not launch, as the launch reported it, and the
/// coordinator it may have left running (its agent id): one that came up
/// wrong and could not be stopped. While one may be running, its run is
/// never folded back, nor its claim released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRefused {
    pub reason: String,
    pub still_running: Option<String>,
}

impl LaunchRefused {
    /// A launch refused with nothing left running.
    pub fn because(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            still_running: None,
        }
    }

    /// A launch refused that may have left coordinator `agent` running.
    pub fn leaving(reason: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            still_running: Some(agent.into()),
        }
    }
}

/// Port: whether a run's swarm is still running, as far as this front end
/// can tell (S4c): a subagent of this session with the run's label still
/// runs. A front end that launches none (the CLI) has none, and a
/// coordinator another session launched is not seen: `end_run` from there
/// relies on its caller to have stopped it.
pub trait RunLiveness: Send + Sync {
    /// The agent id of the coordinator of run `run` of `task` still
    /// running here, if one is: what `agent_cmd kill` stops.
    fn running_coordinator(&self, task: &Slug, run: u32) -> Option<String>;
}

/// Port: how a run's swarm ended, as this front end kept its board (S4c
/// follow-up): read as the run's coordinator left its container, before
/// anything tore the box down, and kept under the coordinator's spawn
/// label, so `end_run` records the run's task items, owners, evidence and
/// cost from it rather than as a swarm that reported nothing. `None` when
/// this front end kept none: one that launches no swarm (the CLI), a run
/// another session launched, a swarm that had not ended when its
/// coordinator left, a board that could not be read whole, a process that
/// restarted between the coordinator's teardown and the run's end, or a
/// teardown that did not know the coordinator's label.
pub trait SwarmRunReports: Send + Sync {
    /// The board of run `run` of `task`'s swarm as it ended, if kept here.
    fn ended_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView>;

    /// The board run `run` of `task`'s swarm had reached as its owner
    /// ended its coordinator while it still ran or was paused (#2674), if
    /// kept here: what a stop, a failure or a loss of the run records,
    /// never a success.
    fn left_run(&self, task: &Slug, run: u32) -> Option<SwarmRunView>;

    /// The board of run `run` of `task`'s swarm has been recorded on the
    /// project board: it need not be kept any longer.
    fn recorded(&self, task: &Slug, run: u32);
}

/// Port: which revisions a run's swarm named in its evidence the board's
/// remote holds (#2666): a revision is on the remote when it names a
/// commit (by its full id, or a prefix of at least 12 hex digits) that one
/// of the remote's branches or tags reaches. A revision a swarm committed
/// in its container and never pushed is not, nor is any text that names
/// no commit. Blocking: it reads the remote.
pub trait RemoteRevisions: Send + Sync {
    /// Those of `revisions` the remote does not hold so, each as given.
    ///
    /// # Errors
    /// The remote could not be reached or read: nothing is judged.
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError>;

    /// How many hex digits a full commit id of this repository has: 40
    /// (SHA-1) or 64 (SHA-256). Read locally.
    ///
    /// # Errors
    /// The repository's object format cannot be read, or is neither.
    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError>;
}

/// Port: the runs this front end's own starts launched (#2655), oldest
/// first: each run of a task whose coordinator a start of this session
/// launched, as `(task, run)`, whether or not it has ended since. A front
/// end that launches none (the CLI) has none. Kept in memory: a process
/// that restarted has forgotten them.
pub trait StartedRuns: Send + Sync {
    fn started(&self) -> Vec<(Slug, u32)>;
}

/// Port: launches a started task's swarm coordinator in a new container
/// (S4c). Its adapter goes through the session's own spawn path, so it
/// launches only what the session itself could spawn, and only while its
/// policy allows spawn. Blocking: it returns once the
/// coordinator is running, or refused.
pub trait CoordinatorLaunch: Send + Sync {
    /// Whether the session may launch one now: its policy allows spawn.
    /// A start asks before it writes anything; the launch asks again.
    fn may_launch(&self) -> bool;

    /// Launches the coordinator `request` describes, giving up once
    /// `cancelled` is set (the call it serves was cancelled): what it began
    /// is stopped, and the launch refused.
    ///
    /// # Errors
    /// The launch's refusal, a cancel's included: nothing is left running,
    /// or the refusal names the coordinator that may be
    /// ([`LaunchRefused::still_running`]).
    fn launch(
        &self,
        request: &CoordinatorRequest,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, LaunchRefused>;
}

/// Port: the board's operations as a front end reaches them (epic #2705
/// B6). The `quecto board` CLI and the agent `board` tool hand each read
/// and checked request to it and answer what it gives back. Requests and
/// answers are owned values and refusals the board's own typed errors: no
/// store handle crosses. The local adapter runs every request on this
/// machine's board; a board server's (B7) will send the board's reads and
/// writes there, while a start (its coordinator's launch) and
/// [`Self::left_runs`] read this session's swarms, so they stay local
/// until B8. Blocking: the board's store, clock and host all block, so
/// callers run it off the async workers.
pub trait BoardOperations: Send + Sync {
    /// What `request` comes to, a listing summarising its tasks as
    /// `summaries` asks, noted as `noting` asks: a noted request gives
    /// back what its use cases saw, answered or refused. A listing is
    /// never refused for its summaries; summaries asked of a request that
    /// is no listing ([`BoardRequest::is_listing`]) are refused as invalid.
    fn execute(
        &self,
        request: &BoardRequest,
        summaries: ListSummaries,
        noting: OpNoting,
    ) -> BoardExecution;

    /// The ends this front end owes as its session ends (#2655): each run
    /// its starts launched that the board still shows open; none for a
    /// front end that launches none.
    ///
    /// # Errors
    /// The board could not be read.
    fn left_runs(&self) -> Result<Vec<RunEnd>, ProjectBoardError>;
}

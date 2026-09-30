//! The read models every member polls (#2277, #1969): the requests and
//! answers of `ReadRunSummary` (`Workbench.summary`), `ReadRunEvents`
//! (`events`), `ListTasks` (`tasks`) and the owner liveness `ReadTask`
//! (`task`) adds, and the rows the board ports read for them.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and
//! every other argument stays the JSON value the caller passed: Python
//! checks a cursor, an offset or a limit with `type(x) is int` at run
//! time.
use serde_json::{Map, Value};

use super::{FileRow, MemberClaimCounts, MemberRow, TaskRow, UsageRow};

/// A row as `dict(row)`: every column in table order.
#[derive(Clone, Debug, PartialEq)]
pub struct DictRow {
    pub columns: Vec<(String, Value)>,
}

impl DictRow {
    /// The value in `column`, when the row has that column.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    /// The dict Python returns, key order included.
    pub fn into_value(self) -> Value {
        Value::Object(self.columns.into_iter().collect::<Map<String, Value>>())
    }
}

/// `Workbench.summary(since=None)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRunSummaryRequest {
    pub actor: String,
    pub since: Value,
}

/// What `summary` answered.
#[derive(Clone, Debug, PartialEq)]
pub enum RunSummary {
    /// `since` is the board's cursor and no owner turned idle since:
    /// `{unchanged, event_cursor, status, next_liveness_check_at}`.
    Unchanged {
        event_cursor: i64,
        /// `run['status']` as stored.
        status: Value,
        next_liveness_check_at: Option<f64>,
        /// What it read (telemetry; Python answers none of it).
        scan: SummaryScan,
    },
    /// The whole summary.
    Full(Box<FullSummary>),
}

/// What a summary read to answer, for telemetry (#2277 review M2):
/// counts and flags only, which Python never answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SummaryScan {
    /// The task owners whose liveness it read, one per owned task: the
    /// liveness watch's, then, for the whole summary, its page's.
    pub owners_scanned: u64,
    /// For a cursor that is the board's: whether an owner turned idle by
    /// the clock alone since it, which defeats the `unchanged` fast path;
    /// `None` for no cursor, or one that is not the board's.
    pub fast_path_defeated: Option<bool>,
    /// For a cursor given: whether the board's cursor is no longer it.
    pub cursor_moved: Option<bool>,
}

/// The whole summary: `dict(run)` (its `constraints` and `criteria`
/// loaded), then the keys Python adds, in its order.
#[derive(Clone, Debug, PartialEq)]
pub struct FullSummary {
    pub run: DictRow,
    pub next_liveness_check_at: Option<f64>,
    pub members: Vec<MemberRow>,
    /// Members whose status is `live` or `reserved`.
    pub usage: i64,
    pub task_count: i64,
    /// The first fifty tasks by id, each with its owner's liveness.
    pub tasks: Vec<TaskRow>,
    pub file_count: i64,
    /// The first fifty reservations by path.
    pub files: Vec<FileRow>,
    pub evidence: Vec<DictRow>,
    pub control_generation: i64,
    pub event_cursor: i64,
    pub counts: SummaryCounts,
    /// What it read (telemetry; Python answers none of it).
    pub scan: SummaryScan,
}

/// `summary()['counts']`: the tasks per status (a `ready` task with an
/// incomplete dependency counted `blocked`), then the #1969 membership
/// counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SummaryCounts {
    pub ready: i64,
    pub claimed: i64,
    pub blocked: i64,
    pub submitted: i64,
    pub completed: i64,
    pub members: MemberClaimCounts,
}

/// `Workbench.events(after=0, limit=25)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRunEventsRequest {
    pub actor: String,
    pub after: Value,
    pub limit: Value,
}

/// A page of the audit history: the events after the cursor (each as
/// `dict(row)`, its detail the stored text), the cursor to read on from,
/// and whether more follow.
#[derive(Clone, Debug, PartialEq)]
pub struct EventPage {
    pub events: Vec<DictRow>,
    pub cursor: i64,
    pub has_more: bool,
}

/// What `tasks` answered: the page, each task with its owner's
/// liveness, and how many task owners it read (telemetry, #2277 review
/// M2).
#[derive(Clone, Debug, PartialEq)]
pub struct TaskPage {
    pub tasks: Vec<TaskRow>,
    pub owners_scanned: u64,
}

/// `Tasks.tasks(offset=0, limit=50)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListTasksRequest {
    pub actor: String,
    pub offset: Value,
    pub limit: Value,
}

/// One task as `summary`'s counts read it (`SELECT id,status,dependencies
/// FROM tasks`): its id and status as stored, its dependencies loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct CountedTask {
    pub id: Value,
    pub status: Value,
    pub dependencies: Value,
}

/// A member's latest board event (`max(time)` of its events), as stored:
/// a REAL the board writes, NULL for events without a time.
#[derive(Clone, Debug, PartialEq)]
pub struct LatestActivity {
    pub actor: String,
    pub latest: Value,
}

/// `Workbench._bootstrap(pid, started, socket, reservation=None)` as
/// `member`: the placeholder when the board holds no run, then the join,
/// answered with the coordinator's summary. The values stay the JSON the
/// member passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapMemberRequest {
    pub member: String,
    pub pid: Value,
    pub started: Value,
    pub socket: Value,
    pub reservation: Value,
}

/// What `_join` answered: the join's branch, and the coordinator's
/// summary every branch ends with, or (#2277 final review L2) the
/// refusal met after the join's writes committed: the activation's after
/// the admission, or the summary's after the activation. A refusal
/// before any write is the call's own, never held here.
#[derive(Clone, Debug, PartialEq)]
pub struct JoinedSummary {
    pub joined: super::Joined,
    pub summary: Result<RunSummary, crate::domain::swarm::BoardError>,
}

/// What `_bootstrap` answered: whether it wrote the placeholder, the
/// join's branch (`None` when a refusal came before the join took one),
/// and the coordinator's summary, or the refusal met after the
/// placeholder or the join committed (#2277 final review L2).
#[derive(Clone, Debug, PartialEq)]
pub struct BootstrappedSummary {
    pub created: bool,
    pub joined: Option<super::Joined>,
    pub summary: Result<RunSummary, crate::domain::swarm::BoardError>,
}

/// What `_run_totals` read (#2313 review M1): the run's own totals, every
/// member's work, as the board holds them. Counts only, and each member's
/// usage row as `usage_report` aggregates it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunTotalsView {
    /// The run row's id as stored, when it is text.
    pub run_id: Option<String>,
    pub task_count: i64,
    /// The tasks by state, as `summary` counts them.
    pub counts: SummaryCounts,
    pub messages: MessageTally,
    /// `usage_report`'s per-member aggregates.
    pub usage: Vec<UsageRow>,
    /// The time of the run's `created` event, when the board holds one.
    pub created_at: Option<f64>,
    /// The board's clock when it was read.
    pub read_at: f64,
}

/// The `messages` rows by what became of them: every row (each was
/// sent), those `consumed` by their recipient, and those `withdrawn`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MessageTally {
    pub sent: i64,
    pub consumed: i64,
    pub withdrawn: i64,
}

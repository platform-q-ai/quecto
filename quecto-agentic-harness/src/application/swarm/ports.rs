//! Capability-local ports of the swarm capability: the lifecycle ports
//! (#1940, #1960) and the coordination board's (#2270).
//!
//! Wire names, positional arguments and persistence schemas are adapter
//! details; every signature here names only domain types and the
//! capability's own DTOs.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use super::dto::{
    AmendedContract, CallMeasure, CompletionState, LaunchIdentity, MemberClaimCounts, MemberRow,
    NewEvidence, NewMember, NewRequestUsage, NewRun, NewTask, PriorEvidence, RunContract,
    RunOwnerRow, RunStatusRow, StoredContract, StoredRequestUsage, TaskRow, TaskUpdate,
    UsageReport,
};
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    BoardError, BoardOpObservation, Member, MemberExit, MemberRecord, ProcessIdentity,
    RunControlAction, RunControlReceipt, RunRecord, RunState, RunStatus, Snapshot,
};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Implementations atomically enforce membership policy before reserving slots.
/// Wire names, positional arguments and persistence schemas are private details.
pub trait CoordinationPort {
    fn snapshot(&self) -> Result<Snapshot, DomainError>;
    fn register_endpoint(&self, endpoint: &str) -> Result<(), DomainError>;
    fn reserve_member(&self, member: &str, token: &str) -> Result<(), DomainError>;
    fn record_launch(
        &self,
        member: &str,
        token: &str,
        process: &ProcessIdentity,
    ) -> Result<(), DomainError>;
    fn confirm_unlaunched(&self, member: &str) -> Result<(), DomainError>;
    /// A member's harness vanished without an authoritative exit observation
    /// (or the coordinator was lost, #1924): ownership is retained and the
    /// run pauses holding `failed`.
    fn quarantine(&self, member: &str) -> Result<(), DomainError>;
    /// The launching harness reaped the member's owned process (#1961): the
    /// member is dead, its active tasks block for the coordinator's
    /// `recover`, and the run keeps going. An orderly exit releases the
    /// member's file reservations; an abrupt one retains them.
    fn confirm_dead(&self, member: &str, exit: MemberExit) -> Result<(), DomainError>;
}

pub trait ProcessObservation {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool;
}

pub trait ProcessControl: Sync {
    /// Cancel this member's detached execution registry independently of turn abort.
    fn cancel_local_executions(&self);
    /// Cancel current jobs while retaining admission for a later resume.
    fn suspend_local_executions(&self, snapshot: &Snapshot);
    /// Suspend only this process; never signal a future turn or another member.
    fn suspend_local_inference(&self, snapshot: &Snapshot);
    fn abort<'a>(&'a self, member: &'a Member) -> PortFuture<'a, bool>;
    /// End the member's harness by delegation (#1939): the shutdown protocol
    /// over the endpoint it registered, the locally owned handle only when
    /// this harness launched it. The member's `ProcessIdentity` is an
    /// observation for liveness, never an authority to signal; a member
    /// reachable neither way is reported failed, not signalled.
    fn terminate<'a>(&'a self, member: &'a Member) -> PortFuture<'a, Result<(), DomainError>>;
}

pub trait Clock {
    fn now_seconds(&self) -> f64;
}

/// What a harness does next once its run has settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementStep {
    /// Nothing is left for this harness: it is the coordinator, or its own
    /// row is no longer live, or the run is not terminal.
    Done,
    /// Still live: settle again on a fresh snapshot and wait for the launcher.
    Wait,
    /// Still live past the grace its launcher's teardown needs: end itself.
    EndSelf,
}

/// Application lifecycle entrypoint, injected by the composition root.
pub trait SwarmLifecycle: std::fmt::Debug + Send + Sync {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError>;
    /// An authoritative exit of a member this harness launched (#1961).
    fn member_exited(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
        member: &str,
        exit: MemberExit,
    ) -> Result<Snapshot, DomainError>;
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
        observation: &'a (dyn ProcessObservation + Sync),
    ) -> PortFuture<'a, Result<(), DomainError>>;
    /// What a harness does next, `elapsed` after it first settled its run.
    fn settlement_step(
        &self,
        snapshot: &Snapshot,
        actor: &str,
        elapsed: std::time::Duration,
        grace: std::time::Duration,
    ) -> SettlementStep;
    /// A member still alive a grace after its run settled ends itself.
    fn settle_overdue<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> PortFuture<'a, Result<(), DomainError>>;
    fn observed_outcome(&self, snapshot: &Snapshot, clock: &dyn Clock) -> RunStatus;
}

/// Supervisor operations remain available without model execution or turn-queue admission.
pub trait SwarmRunControl: Send + Sync {
    fn apply(
        &self,
        action: RunControlAction,
    ) -> PortFuture<'_, Result<RunControlReceipt, DomainError>>;
}

// ─── The coordination board (epic #2265, #2270) ─────────────────────────────
//
// Segregated by role (ADR-0019): a use case bounds only the roles it uses,
// and later slices add roles instead of growing one transaction trait. Every
// role method runs inside the one transaction its `BoardRepository::atomic`
// opened; a store failure is a `BoardError` carrying the board's text
// (`coordination store unavailable or contended: …`).

/// The `run` row.
pub trait BoardRuns {
    /// The run, when the store holds one.
    fn run(&self) -> Result<Option<RunRecord>, BoardError>;
    /// Whether the store holds a run (`_bootstrap`'s `SELECT 1 FROM run`):
    /// no column of it is read.
    fn run_exists(&self) -> Result<bool, BoardError>;
    /// The two columns `create` reads of an existing run (status and
    /// coordinator), as stored, when the store holds one: only those, so a
    /// column it does not read is never decoded.
    fn run_owner(&self) -> Result<Option<RunOwnerRow>, BoardError>;
    /// The columns `_status` reads, as stored, when the store holds a run:
    /// only those, so a column it does not read is never decoded.
    fn run_status(&self) -> Result<Option<RunStatusRow>, BoardError>;
    /// The run's coordinator column alone (`join_process`'s `SELECT
    /// coordinator FROM run`): `None` when the store holds no run, and
    /// `Some(None)` for a coordinator that is NULL or not text.
    fn run_coordinator(&self) -> Result<Option<Option<String>>, BoardError>;
    fn insert_run(&self, run: &NewRun) -> Result<(), BoardError>;
    /// `create` over the setup placeholder: the new contract, and the run
    /// is running.
    fn update_run_contract(&self, contract: &RunContract) -> Result<(), BoardError>;
    /// The run pauses holding `outcome` for the supervisor (#1729).
    fn propose_outcome(&self, outcome: &str, reason: &str) -> Result<(), BoardError>;
    /// The run holds no outcome and no reason (#2273).
    fn clear_outcome(&self) -> Result<(), BoardError>;
    /// `Transaction.set_outcome(status)`: the run's status alone becomes
    /// `status`; its outcome and reason are left as they are.
    fn set_outcome(&self, status: &RunState) -> Result<(), BoardError>;
    /// The run's deadline, in Unix seconds (a REAL).
    fn set_deadline(&self, deadline: f64) -> Result<(), BoardError>;
    /// `Transaction.pause_started`'s read: the `started` value of the
    /// latest `paused` event's detail as stored (NULL when the detail has
    /// none), `None` when no `paused` event exists.
    fn pause_started(&self) -> Result<Option<Value>, BoardError>;
    /// `json.loads(run['criteria'])` (#2273), when the store holds a run.
    fn run_criteria(&self) -> Result<Option<Value>, BoardError>;
    /// The contract `amend` reads before it changes it (#2273), when the
    /// store holds a run.
    fn run_contract(&self) -> Result<Option<StoredContract>, BoardError>;
    /// `amend`'s `UPDATE run SET goal=?,constraints=?,criteria=?`, the two
    /// lists stored with the board's `encode()` (#2273).
    fn amend_contract(&self, contract: &AmendedContract) -> Result<(), BoardError>;
}

/// The `members` rows.
pub trait BoardMembers {
    fn member(&self, id: &str) -> Result<Option<MemberRecord>, BoardError>;
    /// Every member row, in store order.
    fn members(&self) -> Result<Vec<MemberRow>, BoardError>;
    /// Members whose status is `live` or `reserved`.
    fn usage(&self) -> Result<i64, BoardError>;
    /// Members whose status is not `dead` (SQL `status!='dead'`: a NULL
    /// status is not counted), as `create` counts them.
    fn not_dead(&self) -> Result<i64, BoardError>;
    /// `member_claim_counts(coordinator)` (#1969).
    fn claim_counts(&self, coordinator: Option<&str>) -> Result<MemberClaimCounts, BoardError>;
    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError>;
    /// `SELECT * FROM members WHERE id=?` (and `AND reservation=?` when
    /// `reservation` is given) as `dict(row)`. The id and reservation are
    /// the caller's values, bound as Python's `sqlite3` binds them (so `5`
    /// finds the id `'5'` through TEXT affinity, and a NULL matches
    /// nothing); a value it cannot bind is refused naming its parameter.
    fn member_row(
        &self,
        id: &Value,
        reservation: Option<&Value>,
    ) -> Result<Option<MemberRow>, BoardError>;
    /// `Transaction.reserve_member`: a `reserved` row with no process yet,
    /// launched by `launcher` (#1961), the reservation bound as given.
    fn reserve_member(
        &self,
        id: &str,
        reservation: &Value,
        launcher: &str,
    ) -> Result<(), BoardError>;
    /// The member is live in the process `launch` at `socket`, each bound
    /// as given.
    fn activate_member(
        &self,
        id: &Value,
        launch: &LaunchIdentity,
        socket: &Value,
    ) -> Result<(), BoardError>;
    /// The launcher's record of the member's process, bound as given.
    fn record_launch(&self, id: &Value, launch: &LaunchIdentity) -> Result<(), BoardError>;
    /// An admission that was never launched is abandoned: the member is dead.
    fn mark_member_dead_unlaunched(&self, id: &Value) -> Result<(), BoardError>;
    /// The member's endpoint, bound as given; a member without a row is
    /// left as it is.
    fn set_socket(&self, id: &str, socket: &Value) -> Result<(), BoardError>;
    /// `swarm_repository.lost_members` (#1924, #1961, #1969): those of
    /// `members` whose latest `scope_unknown` event is newer than their
    /// latest `activated` one, in one ordered scan of those two event
    /// kinds, in the order given. An event names its member by the text
    /// of its detail's `member`.
    fn lost_members(&self, members: &[&str]) -> Result<Vec<String>, BoardError>;
}

/// The `events` log.
pub trait BoardEvents {
    /// `Store.event`: one row by `actor` at `time` with the encoded detail.
    fn event(&self, actor: &str, time: f64, action: &str, detail: &Value)
    -> Result<(), BoardError>;
    /// The id of the latest `paused` or `resumed` event, `0` for none.
    fn control_generation(&self) -> Result<i64, BoardError>;
}

/// The `tasks` rows (#2272). A task id is the caller's value, bound as
/// Python's `sqlite3` binds it (`"3"` finds task 3 through the column's
/// INTEGER affinity, `true` finds task 1, NULL finds nothing); a value it
/// cannot bind is refused naming its parameter.
pub trait BoardTasks {
    /// `SELECT * FROM tasks WHERE id=?` as `dict(row)`, with `acceptance`,
    /// `dependencies` and `evidence` loaded from their JSON (`_task`
    /// before it derives a blocked status).
    fn task(&self, id: &Value) -> Result<Option<TaskRow>, BoardError>;
    /// `SELECT status FROM tasks WHERE id=?`: the text, `None` for a
    /// missing row or a status that is not text.
    fn task_status(&self, id: &Value) -> Result<Option<String>, BoardError>;
    /// `SELECT * FROM tasks`: every task's id and its loaded dependencies,
    /// in store order (`_dependencies`' graph).
    fn all_task_dependencies(&self) -> Result<Vec<(i64, Value)>, BoardError>;
    fn task_count(&self) -> Result<i64, BoardError>;
    /// A new `ready` task with no evidence; its id.
    fn insert_task(&self, task: &NewTask) -> Result<i64, BoardError>;
    /// The task's dependencies, stored with the board's `encode()`. This and
    /// each task update after it change the one task `id` finds, which the
    /// caller has read in this transaction (the adapter asserts it).
    fn set_task_dependencies(&self, id: &Value, dependencies: &Value) -> Result<(), BoardError>;
    /// The task is `claimed` by `owner` under `token`.
    fn update_task_claim(&self, id: &Value, owner: &str, token: &str) -> Result<(), BoardError>;
    /// The task's status changes as `update` says.
    fn update_task_status(&self, id: &Value, update: &TaskUpdate) -> Result<(), BoardError>;
}

/// The work a new request runs: its result, which the ledger stores.
pub type RequestAction<'a> = dyn FnMut() -> Result<Value, BoardError> + 'a;

/// The request ledger (`Store.retry`, #2272): idempotent retries of a
/// member's request id.
pub trait BoardRequests {
    /// A request `actor` made before replays its stored result when
    /// `payload` encodes to the same text, and is refused with
    /// `request id reused with different payload` otherwise; a new one
    /// runs `action` and stores its result, while the ledger has room.
    /// The replayed result is the stored text as `json.loads` reads it.
    fn retry(
        &self,
        actor: &str,
        request: &str,
        payload: &Value,
        action: &mut RequestAction<'_>,
    ) -> Result<Value, BoardError>;
}

/// The `files` reservations (#2272; S10 extends it).
pub trait BoardFiles {
    /// `DELETE FROM files WHERE task=? AND claim=?`: the reservations made
    /// under that claim of that task, and no other, each bound as given.
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError>;
}

/// The request-usage ledger and the token budget (#2273, #2274). Each
/// method creates `request_usage` and `usage_budget` when absent, as
/// Python's `_usage_schema` does, before it reads or writes them.
pub trait BoardUsage {
    /// `Transaction.usage_report`: `request_usage` and `usage_budget` are
    /// created when absent (Python's statements, in its order), then the
    /// budget, the totals, the per-actor aggregates and the ten latest
    /// observations are read. Without a budget row the budget reads as
    /// `{token_limit: null, strict_unknown: false, warned: false}` and no
    /// row is written.
    fn usage_report(&self) -> Result<UsageReport, BoardError>;
    /// `usage_report()['budget']`: the stored budget payload as
    /// `json.loads` reads it, or the default budget when there is no row.
    fn usage_budget(&self) -> Result<Value, BoardError>;
    /// `INSERT INTO usage_budget VALUES(1,?) ON CONFLICT(id) DO UPDATE`:
    /// the budget row's payload becomes `budget` written by plain
    /// `json.dumps` (insertion order, `", "` and `": "`).
    fn configure_usage_budget(&self, budget: &Value) -> Result<(), BoardError>;
    /// The row of `request_id`, when the ledger holds one.
    fn request_usage(&self, request_id: &str) -> Result<Option<StoredRequestUsage>, BoardError>;
    /// `INSERT INTO request_usage VALUES(?,?,?,?,?,?,?,?,?,?)`, the
    /// payload stored as the text given.
    fn insert_request_usage(&self, usage: &NewRequestUsage) -> Result<(), BoardError>;
    /// The stored payload of `request_id`, which the caller has read in this
    /// transaction, becomes `payload`, the record's `encode()` text.
    fn update_request_usage(&self, request_id: &str, payload: &str) -> Result<(), BoardError>;
    /// `SELECT count(*) FROM request_usage`.
    fn request_usage_count(&self) -> Result<i64, BoardError>;
}

/// The criterion evidence and the task evidence completion reads (#2273;
/// S10 extends it).
pub trait BoardEvidence {
    /// `Transaction.completion_state()`: the run's criteria, every
    /// evidence row, every task as `Transaction.task` reads it, and
    /// whether a file reservation is left.
    fn completion_state(&self) -> Result<CompletionState, BoardError>;
    /// `UPDATE tasks SET evidence=? WHERE id=?` with the evidence written
    /// by plain `json.dumps` (not the board's `encode()`), the id bound as
    /// Python's `sqlite3` binds it.
    fn replace_task_evidence(&self, id: &Value, evidence: &Value) -> Result<(), BoardError>;
    /// `DELETE FROM evidence`: an amended contract keeps no evidence.
    fn delete_all_evidence(&self) -> Result<(), BoardError>;
    /// The row `actor` recorded for `criterion`, bound as given.
    fn prior_evidence(
        &self,
        criterion: &Value,
        actor: &str,
    ) -> Result<Option<PriorEvidence>, BoardError>;
    /// `INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)`.
    fn record_evidence(&self, evidence: &NewEvidence) -> Result<(), BoardError>;
}

/// Every role over one transaction.
pub trait BoardTransaction:
    BoardRuns
    + BoardMembers
    + BoardEvents
    + BoardTasks
    + BoardRequests
    + BoardFiles
    + BoardUsage
    + BoardEvidence
{
}

impl<T> BoardTransaction for T where
    T: BoardRuns
        + BoardMembers
        + BoardEvents
        + BoardTasks
        + BoardRequests
        + BoardFiles
        + BoardUsage
        + BoardEvidence
        + ?Sized
{
}

/// The work one board transaction runs.
pub type BoardWork<'w> = dyn FnMut(&dyn BoardTransaction) -> Result<(), BoardError> + 'w;

/// The board's transaction boundary (`Store.transaction(create)`): `work`
/// runs once inside one transaction, which commits when it succeeds and
/// rolls back when it fails.
pub trait BoardRepository: Send + Sync {
    /// # Errors
    /// `work`'s refusal, unchanged, after rolling back; the store's own
    /// refusal (a missing board, contention).
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError>;
}

/// `uuid.uuid4().hex`: 32 lowercase hex digits, a fresh value per draw.
pub trait IdSource: Send + Sync {
    fn hex32(&self) -> String;
}

/// The board's `encode(value)`: `json.dumps` with sorted keys, compact
/// separators and `ensure_ascii`. The board bounds a JSON argument by the
/// text it would store before storing it.
pub trait BoardEncoding: Send + Sync {
    /// # Errors
    /// A value nested too deep to encode (Python raises `RecursionError`).
    fn encode(&self, value: &Value) -> Result<String, BoardError>;
}

/// Port: measures board calls (#2303), one [`MeteredCall`] per call. The
/// dispatcher opens one around a call only while the event log is on
/// (owner decision T1).
///
/// Nothing is ambient: a call is measured only through its own
/// [`MeteredCall`], which is the repository the call runs its transactions
/// through, so no call's transactions, on this thread or another, are
/// mixed into another's measure.
pub trait BoardCallMeter: Send + Sync {
    /// A fresh measure for one call, sharing nothing with any other.
    fn open(&self) -> Arc<dyn MeteredCall>;
}

/// Port: one board call's measure (#2303), and the repository whose
/// transactions it counts: each transaction begun through it is measured
/// here, and only those.
pub trait MeteredCall: BoardRepository {
    /// What was measured so far; `None` while no transaction has begun,
    /// so nothing was measured.
    fn measure(&self) -> Option<CallMeasure>;
}

/// Port: where each board op's `swarm_op` record goes (#2303): the event
/// log, only while it is on.
pub trait BoardOpLog: Send + Sync {
    /// Appends `observation` synchronously: never on an async runtime,
    /// never blocking on one, never panicking. A failed write is the
    /// adapter's to report and never changes the op's answer.
    fn record(&self, observation: BoardOpObservation);
}

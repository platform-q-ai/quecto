//! Capability-local ports of the swarm capability: the lifecycle ports
//! (#1940, #1960) and the coordination board's (#2270).
//!
//! Wire names, positional arguments and persistence schemas are adapter
//! details; every signature here names only domain types and the
//! capability's own DTOs.
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use super::dto::{
    LaunchIdentity, MemberClaimCounts, MemberRow, NewMember, NewRun, NewTask, RunContract,
    RunOwnerRow, RunStatusRow, TaskRow, TaskUpdate,
};
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    BoardError, Member, MemberExit, MemberRecord, ProcessIdentity, RunControlAction,
    RunControlReceipt, RunRecord, RunStatus, Snapshot,
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
    /// The task's dependencies, stored with the board's `encode()`.
    fn set_task_dependencies(&self, id: &Value, dependencies: &Value) -> Result<(), BoardError>;
    /// The task is `claimed` by `owner` under `token`.
    fn update_task_claim(&self, id: &Value, owner: &str, token: &str) -> Result<(), BoardError>;
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

/// Every role over one transaction.
pub trait BoardTransaction:
    BoardRuns + BoardMembers + BoardEvents + BoardTasks + BoardRequests + BoardFiles
{
}

impl<T> BoardTransaction for T where
    T: BoardRuns + BoardMembers + BoardEvents + BoardTasks + BoardRequests + BoardFiles + ?Sized
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

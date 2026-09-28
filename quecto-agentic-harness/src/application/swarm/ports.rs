//! Capability-local ports of the swarm capability: the lifecycle ports
//! (#1940, #1960) and the coordination board's (#2270).
//!
//! Wire names, positional arguments and persistence schemas are adapter
//! details; every signature here names only domain types and the
//! capability's own DTOs.
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use super::dto::{MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract, RunStatusRow};
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
    /// The columns `_status` reads, as stored, when the store holds a run:
    /// only those, so a column it does not read is never decoded.
    fn run_status(&self) -> Result<Option<RunStatusRow>, BoardError>;
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
}

/// The `events` log.
pub trait BoardEvents {
    /// `Store.event`: one row by `actor` at `time` with the encoded detail.
    fn event(&self, actor: &str, time: f64, action: &str, detail: &Value)
    -> Result<(), BoardError>;
    /// The id of the latest `paused` or `resumed` event, `0` for none.
    fn control_generation(&self) -> Result<i64, BoardError>;
}

/// Every role over one transaction.
pub trait BoardTransaction: BoardRuns + BoardMembers + BoardEvents {}

impl<T: BoardRuns + BoardMembers + BoardEvents + ?Sized> BoardTransaction for T {}

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

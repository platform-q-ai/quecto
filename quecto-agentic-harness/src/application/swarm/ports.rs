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
    AmendedContract, CallMeasure, CompletionState, CountedTask, DictRow, DroppedRecords, FileRow,
    LatestActivity, LaunchIdentity, MemberClaimCounts, MemberRow, MemberStatusRow, MessageRow,
    MessageTally, NewEvidence, NewMember, NewMessage, NewRequestUsage, NewReservation, NewRun,
    NewTask, NotificationCursor, PriorEvidence, RunContract, RunOwnerRow, RunStatusRow,
    ScopeObservation, StoredContract, StoredRequestUsage, TaskRow, TaskUpdate, UsageReport,
    UsageRow, UsageStanding,
};
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    BoardError, BoardOpObservation, Member, MemberExit, MemberRecord, NotificationEvent,
    NotificationState, ProcessIdentity, RunControlAction, RunControlReceipt, RunRecord, RunState,
    RunStatus, Snapshot, SwarmRunSummary,
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
    /// Whether the board holds every column a write transaction adds to an
    /// older board (`ensure_columns`, #2338 final review): a read
    /// transaction, which adds none, answers only from a board that does,
    /// so it answers and leaves the board as the full transaction would.
    fn columns_current(&self) -> Result<bool, BoardError>;
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
    /// `SELECT * FROM run` as `dict(row)` (#2277), every column in table
    /// order, with `constraints` and `criteria` loaded from their JSON, as
    /// `summary` answers it; `None` when the store holds no run.
    fn run_row(&self) -> Result<Option<DictRow>, BoardError>;
    /// `_end_by_loss` on a pause holding no outcome (#2277): `UPDATE run
    /// SET outcome='failed', outcome_reason=?`; the status stays paused.
    fn hold_failed(&self, reason: &str) -> Result<(), BoardError>;
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
    /// The operation gate authorised this transaction's caller as a member
    /// of the run with the op's own access (#2313 review M2). A metered
    /// call notes it, so a refusal the op then makes still records the
    /// caller's role; it runs no statement and changes nothing.
    fn caller_authorized(&self);
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
    /// `SELECT status FROM members WHERE id=?` (#2275): only the status,
    /// so no other column of the row is read or refused. The id is bound
    /// as [`BoardMembers::member_row`] binds it; `None` when no row
    /// matches.
    fn member_status(&self, id: &Value) -> Result<Option<MemberStatusRow>, BoardError>;
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
    /// `UPDATE members SET status='dead' WHERE id=?`: an admission that
    /// was never launched is abandoned, or a death is confirmed (#2277).
    fn mark_member_dead(&self, id: &Value) -> Result<(), BoardError>;
    /// `SELECT launcher FROM members WHERE id=?` (#1961, #2277): `None`
    /// when no row matches, and `Some(None)` for a launcher that is NULL
    /// or not text. The id is bound as [`BoardMembers::member_row`] binds
    /// it.
    fn member_launcher(&self, id: &Value) -> Result<Option<Option<String>>, BoardError>;
    /// `lost_after_activation(member)` (#1961, #2277): whether `member`'s
    /// latest `scope_unknown` event is newer than its latest `activated`
    /// one, in the one ordered scan of those two kinds
    /// [`BoardMembers::lost_members`] runs. An event names `member` when
    /// its detail's `member` equals it by Python's `==`.
    fn lost_after_activation(&self, member: &Value) -> Result<bool, BoardError>;
    /// `SELECT id,status FROM members WHERE id IN (…)` (#1969, #2277): the
    /// status of each of `ids` that has a row (NULL, or a value that is not
    /// text, is `None`), in store order.
    fn member_statuses(&self, ids: &[&str]) -> Result<Vec<(String, Option<String>)>, BoardError>;
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
    /// `SELECT actor, time, detail FROM events WHERE
    /// action='scope_observed' ORDER BY id` (#1961, #2277): every loss
    /// observation, in id order.
    fn scope_observations(&self) -> Result<Vec<ScopeObservation>, BoardError>;
    /// `SELECT actor, max(time) latest FROM events WHERE actor IN (…) GROUP
    /// BY actor` (#1969, #2277): the owner liveness's one grouped scan, the
    /// latest time of each of `actors` that has an event.
    fn latest_activity(&self, actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError>;
    /// `SELECT time FROM events WHERE id=?` (#2277): the event's time as
    /// stored, `None` when no event has that id.
    fn event_time(&self, id: i64) -> Result<Option<Value>, BoardError>;
    /// `SELECT * FROM events WHERE id>? ORDER BY id LIMIT ?` (#2277), each
    /// row as `dict(row)` with its detail the stored text. An `after`
    /// beyond SQLite's integers is refused as the store refuses an integer
    /// it cannot bind.
    fn event_page(&self, after: u64, limit: i64) -> Result<Vec<DictRow>, BoardError>;
    /// `SELECT time FROM events WHERE action='created' ORDER BY id DESC
    /// LIMIT 1` (#2313 review M1): when the run was created, `None` when no
    /// `created` event holds a number.
    fn created_at(&self) -> Result<Option<f64>, BoardError>;
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
    /// `UPDATE tasks SET status='blocked',blocker=? WHERE owner=? AND
    /// status IN ('claimed','blocked','submitted')` (#1961, #2277): every
    /// active task of a member whose death is confirmed, the owner bound
    /// as given.
    fn block_owned_tasks(&self, owner: &Value, blocker: &str) -> Result<(), BoardError>;
    /// `SELECT id FROM tasks ORDER BY id LIMIT ? OFFSET ?` (#2277): the
    /// ids of a page, as stored. An offset beyond SQLite's integers is
    /// refused as the store refuses an integer it cannot bind.
    fn task_ids(&self, offset: u64, limit: i64) -> Result<Vec<Value>, BoardError>;
    /// `SELECT id,status,dependencies FROM tasks` (#2277), in store order,
    /// the dependencies loaded from their JSON.
    fn task_states(&self) -> Result<Vec<CountedTask>, BoardError>;
    /// `SELECT id, owner, status FROM tasks WHERE owner IS NOT NULL AND
    /// status IN ('claimed','blocked','submitted')` (#1969, #2277): the
    /// owner of each held claim, one per task, in store order.
    fn claim_owners(&self) -> Result<Vec<Value>, BoardError>;
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

/// The `files` reservations (#2272, #2275). A task id, a claim token or
/// an ownership token is the caller's value, bound as Python's `sqlite3`
/// binds it.
pub trait BoardFiles {
    /// `DELETE FROM files WHERE task=? AND claim=?`: the reservations made
    /// under that claim of that task, and no other, each bound as given.
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError>;
    /// `SELECT count(*) FROM files`.
    fn file_count(&self) -> Result<i64, BoardError>;
    /// Whether `path` is reserved (`SELECT 1 FROM files WHERE path=?`).
    fn file_reserved(&self, path: &str) -> Result<bool, BoardError>;
    /// `INSERT INTO files VALUES(?,?,?,?,?)`, one row per path in the
    /// order given.
    fn insert_files(&self, reservation: &NewReservation) -> Result<(), BoardError>;
    /// `DELETE FROM files WHERE task=? AND owner=? AND claim=? AND
    /// token=?`: one reservation set of one claim, when it is there.
    fn delete_reservation(
        &self,
        task: &Value,
        owner: &str,
        claim: &Value,
        token: &Value,
    ) -> Result<(), BoardError>;
    /// `SELECT count(*) FROM files WHERE task=?`.
    fn task_file_count(&self, task: &Value) -> Result<i64, BoardError>;
    /// `DELETE FROM files WHERE task=?`: every reservation of the task,
    /// whichever claim made it.
    fn delete_task_files(&self, task: &Value) -> Result<(), BoardError>;
    /// `SELECT * FROM files ORDER BY path LIMIT ? OFFSET ?`, each row as
    /// `dict(row)`. An offset beyond SQLite's integers is refused as the
    /// store refuses an integer it cannot bind.
    fn file_page(&self, offset: u64, limit: i64) -> Result<Vec<FileRow>, BoardError>;
    /// `SELECT count(*) FROM files WHERE owner=?` (#2277), the owner bound
    /// as given.
    fn owner_file_count(&self, owner: &Value) -> Result<i64, BoardError>;
    /// `DELETE FROM files WHERE owner=?` (#2277): every reservation of a
    /// member whose orderly exit is confirmed.
    fn delete_owner_files(&self, owner: &Value) -> Result<(), BoardError>;
}

/// The `messages` rows (#2275, #2276): those revocation writes, and the
/// durable messages `send`, `withdraw`, `inbox` and `ack` read and write.
/// A recipient, a message id or the inbox flag is the caller's (or the
/// board's) value, bound as Python binds it; a value it cannot bind is
/// refused naming its parameter.
pub trait BoardMessages {
    /// `SELECT count(*) FROM messages WHERE recipient=? AND
    /// status='accepted'`: the recipient's unread messages.
    fn inbox_count(&self, recipient: &Value) -> Result<i64, BoardError>;
    /// An `accepted` message from `sender`: its id.
    fn insert_message(
        &self,
        sender: &str,
        recipient: &Value,
        body: &str,
    ) -> Result<i64, BoardError>;
    /// `SELECT * FROM messages WHERE id=?` as `dict(row)`.
    fn message(&self, id: &Value) -> Result<Option<MessageRow>, BoardError>;
    /// `SELECT * FROM messages WHERE id=? AND recipient=?` as `dict(row)`:
    /// the message when it is addressed to `recipient`.
    fn addressed_message(
        &self,
        id: &Value,
        recipient: &str,
    ) -> Result<Option<MessageRow>, BoardError>;
    /// `UPDATE messages SET status=? WHERE id=?`, for a message the caller
    /// has read in this transaction.
    fn set_message_status(&self, id: &Value, status: &str) -> Result<(), BoardError>;
    /// The messages by what became of them (#2313 review M1): `count(*)`,
    /// and the rows whose status is `consumed` or `withdrawn`.
    fn message_tally(&self) -> Result<MessageTally, BoardError>;
    /// `send`'s `INSERT INTO messages(sender,recipient,body,status,
    /// revision,supersedes) VALUES(?,?,?,'accepted',?,?)`: its id.
    fn send_message(&self, message: &NewMessage) -> Result<i64, BoardError>;
    /// `UPDATE messages SET superseded_by=? WHERE id=?`: the message `id`
    /// the caller has just retired is superseded by `successor`.
    fn set_superseded_by(&self, id: &Value, successor: i64) -> Result<(), BoardError>;
    /// `SELECT * FROM messages WHERE recipient=? AND (status='accepted' OR
    /// ?) ORDER BY id LIMIT 100`, each row as `dict(row)`: the flag is
    /// bound as given, so SQLite's truth of it decides.
    fn inbox(
        &self,
        recipient: &str,
        include_consumed: &Value,
    ) -> Result<Vec<MessageRow>, BoardError>;
}

/// The wake frontiers (#2276): each member's `notification_cursors` row
/// (the last event its own hints were sent for) and its `wake_cursors`
/// row (the last generation it accepted a wake for), with the events and
/// the board state the wake policy judges. The actor is the caller's own
/// member id; an event detail is loaded as Python's `json.loads` loads it.
pub trait BoardWakes {
    /// `Transaction.notification_events(actor)`: the events `actor`
    /// recorded after its notification cursor (the stored value bound as
    /// read, `0` without a row), in id order, each naming `actor`.
    fn notification_events(&self, actor: &str) -> Result<Vec<NotificationEvent>, BoardError>;
    /// `advance_notifications(actor)`: `actor`'s notification cursor
    /// becomes the latest event id (`0` for none), by `INSERT OR REPLACE`.
    fn advance_notifications(&self, actor: &str) -> Result<NotificationCursor, BoardError>;
    /// `Transaction.notification_state()`: every task's id, status,
    /// dependencies and owner, and the ids of the unread messages.
    fn notification_state(&self) -> Result<NotificationState, BoardError>;
    /// `CREATE TABLE IF NOT EXISTS wake_cursors`: the table is created
    /// lazily, at a wake claim's start, inside its transaction.
    fn create_wake_cursors(&self) -> Result<(), BoardError>;
    /// `SELECT coalesce(max(id),0) FROM events`: the board's generation.
    fn event_generation(&self) -> Result<i64, BoardError>;
    /// The generation `actor` last accepted a wake for; `None` without a
    /// row. The caller has created `wake_cursors` in this transaction.
    fn wake_cursor(&self, actor: &str) -> Result<Option<i64>, BoardError>;
    /// The events after `previous` up to `generation` that another member
    /// than `actor` recorded (`actor<>?`), in id order.
    fn wake_events(
        &self,
        previous: i64,
        generation: i64,
        actor: &str,
    ) -> Result<Vec<NotificationEvent>, BoardError>;
    /// `INSERT OR REPLACE INTO wake_cursors VALUES(?,?)`.
    fn set_wake_cursor(&self, actor: &str, generation: i64) -> Result<(), BoardError>;
}

/// The shared checkout a board's file reservations name (#2275).
/// Normalising a path reads the filesystem (it follows symlinks), so it is
/// an effect behind this port; the adapter is bound to one checkout.
pub trait CheckoutPaths: Send + Sync {
    /// `str((root / path).resolve().relative_to(root))` with `root` the
    /// checkout resolved: Python's non-strict `Path.resolve()`, which
    /// follows the symlinks that exist and keeps a missing remainder as
    /// written (`..` taken lexically), then the path relative to the root
    /// (`.` for the root itself).
    ///
    /// # Errors
    /// `file must resolve inside the shared checkout` for a path that
    /// resolves outside it, or cannot be resolved.
    fn normalize(&self, path: &str) -> Result<String, BoardError>;
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
    /// `usage_report()['members']` alone (#2313 final review): the
    /// per-member aggregates, reading no request payload and creating no
    /// table; none when the board holds no `request_usage` table yet.
    fn member_usage(&self) -> Result<Vec<UsageRow>, BoardError>;
    /// The budget and the two totals of [`Self::usage_report`] (#2340),
    /// read as it reads them (the tables created when absent, the default
    /// budget without a row) but with one aggregate over the ledger and
    /// nothing else: what the budget decision and the control receipt
    /// need, on every recorded request.
    fn usage_standing(&self) -> Result<UsageStanding, BoardError>;
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

/// The criterion evidence and the task evidence completion reads, and
/// the criterion evidence `evidence` records (#2273).
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
    /// `SELECT * FROM evidence` (#2277), each row as `dict(row)`.
    fn evidence_rows(&self) -> Result<Vec<DictRow>, BoardError>;
}

/// Every role over one transaction.
pub trait BoardTransaction:
    BoardRuns
    + BoardMembers
    + BoardEvents
    + BoardTasks
    + BoardRequests
    + BoardFiles
    + BoardMessages
    + BoardWakes
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
        + BoardMessages
        + BoardWakes
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

    /// Runs `work`, which only reads, in one read transaction (#2338): a
    /// consistent view of the board that takes no write lock, so it
    /// neither waits for a writer's transaction nor holds one off (a
    /// writer's commit waits at most for the read transaction to finish).
    /// Every implementation refuses any write `work` attempts, and never
    /// creates a missing board. It adds no missing column
    /// (`ensure_columns` writes): `work` reads only the board's original
    /// schema, and a caller that may meet an older board falls back to
    /// [`Self::atomic`] on a store refusal. There is no default: a read
    /// that could write would be an `atomic` by another name.
    ///
    /// # Errors
    /// `work`'s refusal; the store's own (a missing board, contention, a
    /// write attempted).
    fn read(&self, work: &mut BoardWork<'_>) -> Result<(), BoardError>;
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

    /// The call's record has a fixed role (the harness's own op, #2313
    /// review nit): the run's roles are not read for it, only its id.
    fn role_fixed(&self);
}

/// Port: where each board op's `swarm_op` record goes (#2303): the event
/// log, only while it is on.
pub trait BoardOpLog: Send + Sync {
    /// Appends `observation` synchronously: never on an async runtime,
    /// never blocking on one, never panicking. A failed write is the
    /// adapter's to report and never changes the op's answer.
    fn record(&self, observation: BoardOpObservation);

    /// Appends a run's `summary` (#2313), as [`Self::record`] appends a
    /// record: synchronously, never panicking, a failed write the
    /// adapter's to report.
    fn summarize(&self, summary: SwarmRunSummary);
}

/// Port: a session's event log a board records in (#2313): a
/// [`BoardOpLog`] that also notes the records dropped before they reached
/// it, and hands over, when a session switch replaces it, the drops it
/// counted but has not noted yet.
pub trait SessionOpLog: BoardOpLog {
    /// Notes `drops` (#2313 review L4: held past their bound before the
    /// session's log opened, or counted by the log this one replaced):
    /// written as the log's drop notes, as synchronously and safely as a
    /// record.
    fn dropped(&self, drops: DroppedRecords);

    /// The drops this log counted and has not noted yet, taken (the counts
    /// restart): handed to the log that replaces it on a session switch
    /// (#2313 review nit), so no drop is lost with the departing log.
    fn take_unnoted(&self) -> DroppedRecords;
}

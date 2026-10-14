//! Board call metering (#2303): what the transactions of one board call
//! waited for, measured in the store where the lock is taken, behind the
//! application's [`BoardCallMeter`] port ([`SqliteBoardCallMeter`]).
//!
//! Nothing here is ambient. Each call the dispatcher meters (only while the
//! event log is on, owner decision T1) opens its own [`MeteredCall`]: one
//! allocation holding the board file (a shared path, not a copy) and the
//! call's own [`Tally`], which is itself the repository the call runs its
//! transactions through. Only those transactions are measured, so nothing
//! another call runs, on this thread or another, is mixed into it; the
//! plain repository takes no timing and keeps SQLite's own busy timeout.
//!
//! While metered, a connection's busy handler is [`busy_callback`] rather
//! than SQLite's built-in 500 ms timeout: the same schedule
//! ([`busy_delay`], `sqliteDefaultBusyCallback`'s), which also notes on
//! the call's tally that it fired and how long it slept, whichever
//! statement found the database busy. The tally reaches the handler as the
//! handler's owned argument, never through shared observer state, and the handler is
//! unregistered ([`unwait_metered`]) before the connection closes.
#[cfg(any(test, feature = "test-support"))]
#[path = "meter_observation.rs"]
pub mod observation;
#[cfg(not(any(test, feature = "test-support")))]
#[path = "meter_observation.rs"]
mod observation;

pub(super) use observation::Phase;
#[cfg(any(test, feature = "test-support"))]
use observation::{AccountedSample, Observation, Scope};
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ffi};

use super::repository::{SqliteBoardRepository, atomic_on, read_on};
use super::store::{BUSY_TIMEOUT, BoardStore};
use super::usage_sums::KeptSums;
use crate::application::swarm::dto::{CallMeasure, RunRoles};
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, BoardWork, MeteredCall};
use crate::domain::swarm::BoardError;
use crate::domain::swarm::telemetry::board_run_id;

/// The production [`BoardCallMeter`]: a metered call per call over the
/// board file of `repository`.
#[derive(Clone, Debug)]
pub struct SqliteBoardCallMeter {
    store: BoardStore,
    sums: KeptSums,
}

impl SqliteBoardCallMeter {
    /// Meters calls on the board file `repository` opens, sharing the
    /// ledger sums it keeps (#2340).
    pub fn new(repository: SqliteBoardRepository) -> Self {
        let (store, sums) = repository.into_parts();
        Self { store, sums }
    }
}

impl BoardCallMeter for SqliteBoardCallMeter {
    fn open(&self) -> Arc<dyn MeteredCall> {
        Arc::new(SqliteMeteredCall {
            store: self.store.clone(),
            sums: self.sums.clone(),
            tally: Tally::new(),
        })
    }
}

/// One call's measure, and the repository that feeds it.
#[derive(Debug)]
struct SqliteMeteredCall {
    store: BoardStore,
    sums: KeptSums,
    tally: Tally,
}

impl BoardRepository for SqliteMeteredCall {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        atomic_on(&self.store, &self.sums, Some(&self.tally), create, work)
    }

    /// A read (#2338), measured as any transaction is: its `BEGIN
    /// DEFERRED` takes no lock, so its lock wait is only the statement's
    /// own, and the busy handler fires only if a writer is committing.
    fn read(&self, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        read_on(&self.store, &self.sums, Some(&self.tally), work)
    }
}

impl MeteredCall for SqliteMeteredCall {
    fn measure(&self) -> Option<CallMeasure> {
        let measure = self.tally.held();
        (measure.transactions > 0).then(|| measure.clone())
    }

    fn role_fixed(&self) {
        self.tally.roles_wanted.store(false, Ordering::Release);
    }
}

#[cfg(any(test, feature = "test-support"))]
fn next_identity(sequence: &std::sync::atomic::AtomicU64) -> u64 {
    sequence
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .expect("meter identity space exhausted")
}

/// What one metered call has measured so far.
pub(super) struct Tally {
    measure: Mutex<CallMeasure>,
    #[cfg(any(test, feature = "test-support"))]
    observation: Option<CallObservation>,
    /// Whether the call's record names its caller's role, so the run's
    /// roles are read for it: not for a call whose role is fixed.
    roles_wanted: AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
struct CallObservation {
    call_id: u128,
    connections: std::sync::atomic::AtomicU64,
    observer: Arc<dyn Fn(Observation) + Send + Sync>,
}

impl Tally {
    /// A measure that has measured nothing.
    pub(super) fn new() -> Self {
        Self {
            measure: Mutex::new(CallMeasure::default()),
            #[cfg(any(test, feature = "test-support"))]
            observation: None,
            roles_wanted: AtomicBool::new(true),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn observe(&mut self, observer: Arc<dyn Fn(Observation) + Send + Sync>) {
        self.observation = Some(CallObservation {
            call_id: uuid::Uuid::new_v4().as_u128(),
            connections: std::sync::atomic::AtomicU64::new(1),
            observer,
        });
    }

    /// The measure, also after a panic elsewhere left the lock poisoned:
    /// nothing is ever left half-updated under it.
    fn held(&self) -> MutexGuard<'_, CallMeasure> {
        self.measure.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A transaction began, having waited `wait` for its lock.
    pub(super) fn lock_waited(&self, wait: Duration) {
        let mut measure = self.held();
        measure.transactions = measure.transactions.saturating_add(1);
        measure.lock_wait = measure.lock_wait.saturating_add(wait);
    }

    /// A transaction's `COMMIT` took `took` (#2340).
    pub(super) fn committed(&self, took: Duration) {
        let mut measure = self.held();
        measure.commit = measure.commit.saturating_add(took);
    }

    /// Whether no run id, or (for a call whose role is not fixed) no run
    /// roles, have been found yet: only then is a run row the op read
    /// noted, and only then does a transaction that found them not read
    /// the run row for them.
    pub(super) fn wants_run(&self) -> bool {
        let roles_wanted = self.roles_wanted.load(Ordering::Acquire);
        let measure = self.held();
        measure.run_id.is_none() || (roles_wanted && measure.run_roles.is_none())
    }

    /// A transaction found the run `id` (or none) and its `roles` (`None`
    /// when the row it read does not hold them). The id is kept when no
    /// run id was found before, and only when it is one the board generates
    /// ([`board_run_id`], #2303 round-4 review L3); an id edited from
    /// outside is never recorded. The roles are kept when none were found
    /// before (#2303 reconcile), whatever the id.
    pub(super) fn run_seen(&self, id: Option<&str>, roles: Option<RunRoles>) {
        let mut measure = self.held();
        if let Some(id) = id.filter(|id| board_run_id(id)) {
            if measure.run_id.is_none() {
                measure.run_id = Some(id.to_owned());
            }
        }
        if measure.run_roles.is_none() {
            measure.run_roles = roles;
        }
    }

    /// The operation gate authorised the call's caller as a member of the
    /// run (#2313 review M2).
    pub(super) fn caller_authorized(&self) {
        self.held().authorized = true;
    }
}

impl std::fmt::Debug for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tally")
            .field("measure", &*self.held())
            .finish_non_exhaustive()
    }
}

impl SqliteBoardCallMeter {
    /// A per-call adapter seam, intentionally absent from the application port.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_observed(
        &self,
        observer: Arc<dyn Fn(Observation) + Send + Sync>,
    ) -> Arc<dyn MeteredCall> {
        let mut tally = Tally::new();
        tally.observe(observer);
        Arc::new(SqliteMeteredCall {
            store: self.store.clone(),
            sums: self.sums.clone(),
            tally,
        })
    }
}

/// Owns the connection and its boxed FFI argument. Unregisters on every path,
/// including unwind, before either allocation can be destroyed.
pub(super) struct MeteredConnection<'a> {
    connection: Option<Connection>,
    handler: Option<Box<Handler<'a>>>,
}

struct Handler<'a> {
    tally: &'a Tally,
    #[cfg(any(test, feature = "test-support"))]
    observation: Option<HandlerObservation>,
}

#[cfg(any(test, feature = "test-support"))]
struct HandlerObservation {
    scope: Scope,
    phase: Arc<AtomicU8>,
    actual: Mutex<(Phase, Duration)>,
}

#[cfg(any(test, feature = "test-support"))]
impl HandlerObservation {
    fn scope(&self) -> Scope {
        let phase = match self.phase.load(Ordering::Acquire) {
            1 => Phase::Begin,
            2 => Phase::Work,
            3 => Phase::Commit,
            _ => Phase::Setup,
        };
        Scope {
            phase,
            ..self.scope
        }
    }
}

impl Handler<'_> {
    #[cfg(any(test, feature = "test-support"))]
    fn notify(&self, event: Observation) {
        if let Some(observation) = &self.tally.observation {
            (observation.observer)(event);
        }
    }

    fn account(&self, _count: c_int, slept: Duration) {
        {
            let mut measure = self.tally.held();
            measure.busy_wait = measure.busy_wait.saturating_add(slept);
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(observation) = &self.observation {
            let scope = observation.scope();
            let actual = {
                let mut actual = observation
                    .actual
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if actual.0 == scope.phase {
                    actual.1 = actual.1.saturating_add(slept);
                } else {
                    *actual = (scope.phase, slept);
                }
                actual.1
            };
            // Both mutexes released; immutable actual sleep is already accounted.
            self.notify(Observation::Accounted(AccountedSample {
                scope,
                callback: _count,
                actual,
            }));
        }
    }

    fn busy(&self, count: c_int) -> bool {
        self.tally.held().busy = true;
        if let Some(delay) = busy_delay(count) {
            let started = Instant::now();
            std::thread::sleep(delay);
            self.account(count, started.elapsed());
            true
        } else {
            false
        }
    }
}

impl<'a> MeteredConnection<'a> {
    pub(super) fn new(connection: Connection, tally: Option<&'a Tally>) -> rusqlite::Result<Self> {
        let mut owned = Self {
            connection: Some(connection),
            handler: None,
        };
        if let Some(tally) = tally {
            // SAFETY: this connection is live and owned until unregister.
            let handle = unsafe {
                owned
                    .connection
                    .as_ref()
                    .expect("owned connection")
                    .handle()
            };
            let mut handler = Box::new(Handler {
                tally,
                #[cfg(any(test, feature = "test-support"))]
                observation: tally.observation.as_ref().map(|observation| {
                    let identity = next_identity(&observation.connections);
                    HandlerObservation {
                        scope: Scope {
                            call: observation.call_id,
                            connection: identity,
                            transaction: identity,
                            phase: Phase::Setup,
                        },
                        phase: Arc::new(AtomicU8::new(0)),
                        actual: Mutex::new((Phase::Setup, Duration::ZERO)),
                    }
                }),
            });
            let argument = std::ptr::from_mut(&mut *handler).cast::<c_void>();
            // SQLite uses the argument synchronously on this connection's thread.
            // SAFETY: the live handle and stable boxed argument are owned until unregister.
            let code = unsafe { ffi::sqlite3_busy_handler(handle, Some(busy_callback), argument) };
            if code == ffi::SQLITE_OK {
                owned.handler = Some(handler);
            } else {
                return Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None));
            }
        } else {
            owned
                .connection
                .as_ref()
                .expect("owned connection")
                .busy_timeout(BUSY_TIMEOUT)?;
        }
        Ok(owned)
    }

    pub(super) fn phase(&self, _phase: Phase) {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(handler) = &self.handler {
            if let Some(observation) = &handler.observation {
                let value = match _phase {
                    Phase::Setup => 0,
                    Phase::Begin => 1,
                    Phase::Work => 2,
                    Phase::Commit => 3,
                };
                observation.phase.store(value, Ordering::Release);
                if _phase == Phase::Begin {
                    handler.notify(Observation::Begin(observation.scope()));
                }
            }
        }
    }

    pub(super) fn phase_control(&self) -> Option<Arc<AtomicU8>> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(handler) = &self.handler {
            if let Some(observation) = &handler.observation {
                return Some(observation.phase.clone());
            }
        }
        None
    }

    fn unregister(&mut self) {
        if self.handler.is_some() {
            let result = unwait_metered(self.connection.as_ref().expect("handler owns connection"));
            assert!(
                result.is_ok(),
                "clearing a live busy handler cannot fail: {result:?}"
            );
            self.handler = None;
        }
    }

    pub(super) fn close(mut self) -> rusqlite::Result<()> {
        self.unregister();
        self.connection
            .take()
            .expect("owned connection")
            .close()
            .map_err(|(_, error)| error)
    }
}

impl std::ops::Deref for MeteredConnection<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().expect("owned connection")
    }
}
impl std::ops::DerefMut for MeteredConnection<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection.as_mut().expect("owned connection")
    }
}
impl Drop for MeteredConnection<'_> {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// Removes [`busy_callback`] from `connection`, before it closes (#2303
/// round-3 review L6): once this returns, SQLite holds no pointer to the
/// tally, so nothing can reach it through the connection however long the
/// connection outlives it.
pub(super) fn unwait_metered(connection: &Connection) -> rusqlite::Result<()> {
    // The handle is the open connection's own, used on this thread while
    // `connection` is borrowed.
    // SAFETY: a live handle, read and not kept.
    let handle = unsafe { connection.handle() };
    // A null handler clears the connection's busy handler, and its null
    // argument is never read (`sqlite3_busy_handler`: "a NULL busy handler
    // ... returns SQLITE_BUSY immediately").
    // SAFETY: a live handle; no pointer is handed to SQLite.
    let code = unsafe { ffi::sqlite3_busy_handler(handle, None, std::ptr::null_mut()) };
    match code {
        ffi::SQLITE_OK => Ok(()),
        code => Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)),
    }
}

/// SQLite's busy handler while metered: accounting on the handler it
/// was installed with. A panic never crosses into SQLite: it gives up.
unsafe extern "C" fn busy_callback(argument: *mut c_void, count: c_int) -> c_int {
    // The owning MeteredConnection installed a stable boxed Handler, and
    // unregisters before either the argument or connection is dropped.
    // SAFETY: the pointer is valid and only read through.
    if let Some(handler) = unsafe { argument.cast::<Handler<'_>>().as_ref() } {
        let retry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.busy(count)));
        match retry {
            Ok(retry) => c_int::from(retry),
            Err(payload) => {
                // Arbitrary payload Drop can panic too. Deliberately leak only this
                // exceptional payload rather than run user code outside containment.
                std::mem::forget(payload);
                0
            }
        }
    } else {
        0
    }
}

/// `sqliteDefaultBusyCallback`'s delays and their running totals, in ms.
const DELAYS_MS: [u64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
const TOTALS_MS: [u64; 12] = [0, 1, 3, 8, 18, 33, 53, 78, 103, 128, 178, 228];

/// SQLite's default busy handler's wait before retry `count` under
/// [`BUSY_TIMEOUT`], or `None` once the timeout is spent.
fn busy_delay(count: c_int) -> Option<Duration> {
    let timeout = u64::try_from(BUSY_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
    let count = usize::try_from(count).ok()?;
    let (delay, prior) = match (DELAYS_MS.get(count), TOTALS_MS.get(count)) {
        (Some(&delay), Some(&prior)) => (delay, prior),
        _ => {
            let beyond = u64::try_from(count - DELAYS_MS.len() + 1).unwrap_or(u64::MAX);
            (100, 228_u64.saturating_add(beyond.saturating_mul(100)))
        }
    };
    let delay = delay.min(timeout.saturating_sub(prior));
    (delay > 0).then(|| Duration::from_millis(delay))
}

#[cfg(test)]
#[path = "meter_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "meter_boundary_tests.rs"]
mod boundary_tests;

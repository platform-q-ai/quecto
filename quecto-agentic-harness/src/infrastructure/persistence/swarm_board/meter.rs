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
//! handler's own argument, never through shared state, and the handler is
//! unregistered ([`unwait_metered`]) before the connection closes.
use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ffi};

use super::repository::{SqliteBoardRepository, atomic_on, read_on};
use super::store::{BUSY_TIMEOUT, BoardStore};
use crate::application::swarm::dto::{CallMeasure, RunRoles};
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, BoardWork, MeteredCall};
use crate::domain::swarm::BoardError;
use crate::domain::swarm::telemetry::board_run_id;

/// The production [`BoardCallMeter`]: a metered call per call over the
/// board file of `repository`.
#[derive(Clone, Debug)]
pub struct SqliteBoardCallMeter {
    store: BoardStore,
}

impl SqliteBoardCallMeter {
    /// Meters calls on the board file `repository` opens.
    pub fn new(repository: SqliteBoardRepository) -> Self {
        Self {
            store: repository.into_store(),
        }
    }
}

impl BoardCallMeter for SqliteBoardCallMeter {
    fn open(&self) -> Arc<dyn MeteredCall> {
        Arc::new(SqliteMeteredCall {
            store: self.store.clone(),
            tally: Tally::new(),
        })
    }
}

/// One call's measure, and the repository that feeds it.
#[derive(Debug)]
struct SqliteMeteredCall {
    store: BoardStore,
    tally: Tally,
}

impl BoardRepository for SqliteMeteredCall {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        atomic_on(&self.store, Some(&self.tally), create, work)
    }

    /// A read (#2338), measured as any transaction is: its `BEGIN
    /// DEFERRED` takes no lock, so its lock wait is only the statement's
    /// own, and the busy handler fires only if a writer is committing.
    fn read(&self, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        read_on(&self.store, Some(&self.tally), work)
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

/// What one metered call has measured so far.
#[derive(Debug)]
pub(super) struct Tally {
    measure: Mutex<CallMeasure>,
    /// Whether the call's record names its caller's role, so the run's
    /// roles are read for it: not for a call whose role is fixed.
    roles_wanted: AtomicBool,
}

impl Tally {
    /// A measure that has measured nothing.
    pub(super) fn new() -> Self {
        Self {
            measure: Mutex::new(CallMeasure::default()),
            roles_wanted: AtomicBool::new(true),
        }
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

    /// The busy handler's step `count`: notes that it fired, then waits as
    /// SQLite's default handler does, giving up once the timeout is spent,
    /// and adds what it slept to the busy wait.
    fn busy(&self, count: c_int) -> bool {
        self.held().busy = true;
        match busy_delay(count) {
            Some(delay) => {
                let slept = Instant::now();
                std::thread::sleep(delay);
                let slept = slept.elapsed();
                let mut measure = self.held();
                measure.busy_wait = measure.busy_wait.saturating_add(slept);
                true
            }
            None => false,
        }
    }
}

/// Installs [`busy_callback`] on `connection`, reporting to `tally`.
///
/// `tally` must outlive every statement `connection` runs: the store opens
/// and closes the connection inside one transaction, while the metered
/// call that owns the tally is borrowed, and unregisters the handler
/// ([`unwait_metered`]) before it closes the connection.
pub(super) fn wait_metered(connection: &Connection, tally: &Tally) -> rusqlite::Result<()> {
    let argument = std::ptr::from_ref(tally).cast_mut().cast::<c_void>();
    // The handle is the open connection's own, used on this thread while
    // `connection` is borrowed.
    // SAFETY: a live handle, read and not kept.
    let handle = unsafe { connection.handle() };
    // `argument` points at a `Tally` that outlives the connection (see
    // above), and `busy_callback` only reads through it.
    // SAFETY: a live handle, and a pointer valid for the connection's life.
    let code = unsafe { ffi::sqlite3_busy_handler(handle, Some(busy_callback), argument) };
    match code {
        ffi::SQLITE_OK => Ok(()),
        code => Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)),
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

/// SQLite's busy handler while metered: [`Tally::busy`] on the tally it
/// was installed with. A panic never crosses into SQLite: it gives up.
unsafe extern "C" fn busy_callback(argument: *mut c_void, count: c_int) -> c_int {
    // `wait_metered` installed this handler with a pointer to a live
    // `Tally`, which outlives the connection that calls it.
    // SAFETY: the pointer is valid and only read through.
    let tally = unsafe { &*argument.cast_const().cast::<Tally>() };
    let retry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tally.busy(count)));
    c_int::from(retry.unwrap_or(false))
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

//! Board call metering (#2303): what the transactions of one board call
//! waited for, measured in the store where the lock is taken, behind the
//! application's [`BoardCallMeter`] port ([`SqliteBoardCallMeter`]).
//!
//! Nothing here is ambient. Each call the dispatcher meters (only while the
//! event log is on, owner decision T1) opens its own [`MeteredCall`]: a
//! [`SqliteBoardRepository`] built for that call alone, carrying the call's
//! own [`Tally`]. Only transactions begun through that repository are
//! measured, so nothing another call runs, on this thread or another, is
//! mixed into it; a repository built without a tally takes no timing and
//! keeps SQLite's own busy timeout.
//!
//! A measure can nest in another ([`MeteredCall::nested`]): the inner
//! tally is its own, and everything it measures is added to the outer one
//! too, so an outer call's lock wait includes every wait it sat through.
//!
//! While metered, a connection's busy handler is [`busy_callback`] rather
//! than SQLite's built-in 500 ms timeout: the same schedule
//! ([`busy_delay`], `sqliteDefaultBusyCallback`'s), which also notes on
//! the call's tally that it fired and how long it slept, whichever
//! statement found the database busy. The tally reaches the handler as the
//! handler's own argument, never through shared state.
use std::ffi::{c_int, c_void};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ffi};

use super::repository::SqliteBoardRepository;
use super::store::BUSY_TIMEOUT;
use crate::application::swarm::dto::CallMeasure;
use crate::application::swarm::ports::{BoardCallMeter, BoardRepository, MeteredCall};

/// The production [`BoardCallMeter`]: a metered repository per call over
/// the board file of `repository`.
#[derive(Clone, Debug)]
pub struct SqliteBoardCallMeter {
    repository: SqliteBoardRepository,
}

impl SqliteBoardCallMeter {
    /// Meters calls on the board file `repository` opens.
    pub fn new(repository: SqliteBoardRepository) -> Self {
        Self { repository }
    }
}

impl BoardCallMeter for SqliteBoardCallMeter {
    fn open(&self) -> Box<dyn MeteredCall> {
        Box::new(SqliteMeteredCall::new(&self.repository, None))
    }
}

/// One call's measure and the repository that feeds it.
struct SqliteMeteredCall {
    tally: Arc<Tally>,
    repository: Arc<SqliteBoardRepository>,
}

impl SqliteMeteredCall {
    fn new(over: &SqliteBoardRepository, outer: Option<Arc<Tally>>) -> Self {
        let tally = Arc::new(Tally {
            measure: Mutex::new(CallMeasure::default()),
            outer,
        });
        Self {
            repository: Arc::new(over.metered(tally.clone())),
            tally,
        }
    }
}

impl MeteredCall for SqliteMeteredCall {
    fn repository(&self) -> Arc<dyn BoardRepository> {
        self.repository.clone()
    }

    fn nested(&self) -> Box<dyn MeteredCall> {
        Box::new(Self::new(&self.repository, Some(self.tally.clone())))
    }

    fn measure(&self) -> Option<CallMeasure> {
        let measure = self.tally.held().clone();
        (measure.transactions > 0).then_some(measure)
    }
}

/// What one metered call has measured so far, and the measure it is
/// nested in, which everything recorded here reaches too.
#[derive(Debug)]
pub(super) struct Tally {
    measure: Mutex<CallMeasure>,
    outer: Option<Arc<Tally>>,
}

impl Tally {
    /// The measure, also after a panic elsewhere left the lock poisoned:
    /// nothing is ever left half-updated under it.
    fn held(&self) -> MutexGuard<'_, CallMeasure> {
        self.measure.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies `change` to this measure and every one it is nested in.
    fn record(&self, change: impl Fn(&mut CallMeasure)) {
        let mut next = Some(self);
        while let Some(tally) = next {
            change(&mut tally.held());
            next = tally.outer.as_deref();
        }
    }

    /// A transaction began, having waited `wait` for its lock.
    pub(super) fn lock_waited(&self, wait: Duration) {
        self.record(|measure| {
            measure.transactions = measure.transactions.saturating_add(1);
            measure.lock_wait = measure.lock_wait.saturating_add(wait);
        });
    }

    /// Whether this measure, or one it is nested in, has found no run id
    /// yet: only then does a transaction read it.
    pub(super) fn wants_run_id(&self) -> bool {
        let mut next = Some(self);
        while let Some(tally) = next {
            if tally.held().run_id.is_none() {
                return true;
            }
            next = tally.outer.as_deref();
        }
        false
    }

    /// A transaction found the run `id` (or none): kept wherever no run id
    /// was found before.
    pub(super) fn run_seen(&self, id: Option<&str>) {
        if let Some(id) = id {
            self.record(|measure| {
                if measure.run_id.is_none() {
                    measure.run_id = Some(id.to_owned());
                }
            });
        }
    }

    /// The busy handler's step `count`: notes that it fired, then waits as
    /// SQLite's default handler does, giving up once the timeout is spent,
    /// and adds what it slept to the busy wait.
    fn busy(&self, count: c_int) -> bool {
        self.record(|measure| measure.busy = true);
        match busy_delay(count) {
            Some(delay) => {
                let slept = Instant::now();
                std::thread::sleep(delay);
                let slept = slept.elapsed();
                self.record(|measure| measure.busy_wait = measure.busy_wait.saturating_add(slept));
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
/// repository that owns the tally is borrowed.
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

thread_local! {
    static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a call on this thread is being metered.
pub(super) fn active() -> bool {
    ACTIVE.with(std::cell::Cell::get)
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

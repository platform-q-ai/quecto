//! Board call metering (#2303): what the transactions of one board call
//! waited for, measured in the store where the lock is taken.
//!
//! A call is metered only inside [`metered`], which the dispatcher opens
//! only when the event log is switched on (owner decision T1): outside it
//! the store takes no timing and installs no handler of its own. The
//! measure is confined to the calling thread and to the one call, and is
//! handed back as `metered`'s result: a board call runs its use case
//! synchronously on one thread, so nothing another call measures, on this
//! thread or another, is ever mixed into it. Scopes nest, each restoring
//! the one it interrupted.
//!
//! While metered, the store's busy handler is [`metered_busy`] rather than
//! SQLite's built-in 500 ms timeout: the same schedule ([`busy_delay`],
//! `sqliteDefaultBusyCallback`'s), which also notes that it fired.
use std::cell::RefCell;
use std::time::Duration;

use super::store::BUSY_TIMEOUT;

/// What one board call's transactions measured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallMeter {
    /// From `BEGIN IMMEDIATE` issued to acquired (or given up), summed
    /// over the call's transactions.
    pub lock_wait: Duration,
    /// Whether the busy handler fired in any of them.
    pub busy: bool,
    /// The run id the call's first transaction to find one found.
    pub run_id: Option<String>,
}

thread_local! {
    static CURRENT: RefCell<Option<CallMeter>> = const { RefCell::new(None) };
}

/// Restores the scope a [`metered`] call interrupted, also on unwind.
struct Scope(Option<Option<CallMeter>>);

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(outer) = self.0.take() {
            CURRENT.with(|current| *current.borrow_mut() = outer);
        }
    }
}

/// Runs `work`, metering the board transactions it runs on this thread.
pub fn metered<T>(work: impl FnOnce() -> T) -> (T, CallMeter) {
    let scope = Scope(Some(
        CURRENT.with(|current| current.replace(Some(CallMeter::default()))),
    ));
    let value = work();
    let meter = CURRENT.with(|current| current.borrow_mut().take());
    debug_assert!(
        meter.is_some(),
        "a scope's meter stays in place until it closes"
    );
    drop(scope);
    (value, meter.unwrap_or_default())
}

/// Whether a call on this thread is being metered.
pub fn active() -> bool {
    CURRENT.with(|current| current.borrow().is_some())
}

fn update(change: impl FnOnce(&mut CallMeter)) {
    CURRENT.with(|current| {
        if let Some(meter) = current.borrow_mut().as_mut() {
            change(meter);
        }
    });
}

/// A transaction waited `wait` for its lock.
pub(super) fn lock_waited(wait: Duration) {
    update(|meter| meter.lock_wait += wait);
}

/// Whether the call is metered and has found no run id yet: only then
/// does a transaction read it.
pub(super) fn wants_run_id() -> bool {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .is_some_and(|meter| meter.run_id.is_none())
    })
}

/// A transaction found the run `id` (or none).
pub(super) fn run_seen(id: Option<String>) {
    update(|meter| meter.run_id = id);
}

/// The store's busy handler while metered: notes that it fired, then waits
/// as SQLite's default handler does, giving up once the timeout is spent.
pub(super) fn metered_busy(count: i32) -> bool {
    update(|meter| meter.busy = true);
    match busy_delay(count) {
        Some(delay) => {
            std::thread::sleep(delay);
            true
        }
        None => false,
    }
}

/// `sqliteDefaultBusyCallback`'s delays and their running totals, in ms.
const DELAYS_MS: [u64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
const TOTALS_MS: [u64; 12] = [0, 1, 3, 8, 18, 33, 53, 78, 103, 128, 178, 228];

/// SQLite's default busy handler's wait before retry `count` under
/// [`BUSY_TIMEOUT`], or `None` once the timeout is spent.
pub fn busy_delay(count: i32) -> Option<Duration> {
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

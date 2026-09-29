//! Board call metering (#2303). RED STUB.
use std::time::Duration;

/// What one board call's transactions measured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallMeter {
    pub lock_wait: Duration,
    pub busy: bool,
    pub run_id: Option<String>,
}

/// Runs `work`, metering the board transactions it runs on this thread.
pub fn metered<T>(work: impl FnOnce() -> T) -> (T, CallMeter) {
    (work(), CallMeter::default())
}

/// Whether a call on this thread is being metered.
pub fn active() -> bool {
    false
}

/// SQLite's default busy handler's wait before retry `count`.
pub fn busy_delay(_count: i32) -> Option<Duration> {
    None
}

#[cfg(test)]
#[path = "meter_tests.rs"]
mod tests;

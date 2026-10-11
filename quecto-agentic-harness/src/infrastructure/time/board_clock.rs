//! The project board's clock (#2476): this machine's time, to the second.
//! It has no outside reference yet: a follow-up gives it the remote's time
//! (the HTTPS `Date` header) before board writes are exposed.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::application::project_board::ports::BoardClock;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The pause before a retried write grows by this much each attempt, plus
/// up to this much jitter.
const PAUSE_STEP: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBoardClock;

impl BoardClock for SystemBoardClock {
    fn now(&self) -> Timestamp {
        let text = humantime::format_rfc3339_seconds(SystemTime::now()).to_string();
        Timestamp::parse(&text).expect("the system clock reads a time between 1970 and 9999")
    }

    fn reference(&self) -> Option<Timestamp> {
        None
    }

    fn pause(&self, attempt: usize) {
        std::thread::sleep(pause_for(attempt));
    }
}

/// `attempt - 1` steps plus a jitter from the clock's sub-second digits.
pub fn pause_for(attempt: usize) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let jitter = Duration::from_nanos(u64::from(nanos) % PAUSE_STEP.as_nanos() as u64);
    let steps = u32::try_from(attempt.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(10);
    PAUSE_STEP * steps + jitter
}

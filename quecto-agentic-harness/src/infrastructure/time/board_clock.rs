//! The project board's clock (#2476): this machine's time, to the second.
//! It has no outside reference yet: a follow-up gives it the remote's time
//! (the HTTPS `Date` header) before board writes are exposed.
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::application::project_board::ports::{BoardClock, BoardClockError};
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The pause before a retried write grows by this much each attempt, plus
/// up to this much jitter.
const PAUSE_STEP: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub struct SystemBoardClock {
    jitter: Mutex<StdRng>,
}

impl Default for SystemBoardClock {
    fn default() -> Self {
        Self {
            jitter: Mutex::new(StdRng::from_entropy()),
        }
    }
}

impl SystemBoardClock {
    /// A clock whose pauses are jittered from `seed`: the same seed gives
    /// the same pauses.
    pub fn seeded(_seed: u64) -> Self {
        Self::default()
    }

    /// The pause before attempt `attempt`: `attempt - 1` steps (at most
    /// ten) plus a jitter under one step.
    pub fn pause_for(&self, attempt: usize) -> Duration {
        drop(self.jitter.lock());
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.subsec_nanos());
        let jitter = Duration::from_nanos(u64::from(nanos) % PAUSE_STEP.as_nanos() as u64);
        let steps = u32::try_from(attempt.saturating_sub(1))
            .unwrap_or(u32::MAX)
            .min(10);
        PAUSE_STEP * steps + jitter
    }
}

/// `time` to the second, as the board spells it.
///
/// # Errors
/// A time before 1970 or past 9999.
pub fn timestamp_at(time: SystemTime) -> Result<Timestamp, BoardClockError> {
    let text = humantime::format_rfc3339_seconds(time).to_string();
    Ok(Timestamp::parse(&text).expect("the system clock reads a time between 1970 and 9999"))
}

impl BoardClock for SystemBoardClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        timestamp_at(SystemTime::now())
    }

    fn reference(&self) -> Option<Timestamp> {
        None
    }

    fn pause(&self, attempt: usize) {
        std::thread::sleep(self.pause_for(attempt));
    }
}

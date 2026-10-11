//! The project board's clock (#2476): this machine's time, to the second,
//! with no outside reference of its own. [`remote_time`](super::remote_time)
//! gives it one (#2491): the board remote's HTTPS `Date`.
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::application::project_board::ports::{BoardClock, BoardClockError};
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The pause before a retried write grows by this much each attempt, plus
/// up to this much jitter.
const PAUSE_STEP: Duration = Duration::from_millis(50);
/// A pause grows for at most this many steps.
const MAX_STEPS: u32 = 10;

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
    pub fn seeded(seed: u64) -> Self {
        Self {
            jitter: Mutex::new(StdRng::seed_from_u64(seed)),
        }
    }

    /// The pause before attempt `attempt`: `attempt - 1` steps (at most
    /// ten) plus a jitter under one step, drawn afresh each time.
    pub fn pause_for(&self, attempt: usize) -> Duration {
        let step = u64::try_from(PAUSE_STEP.as_nanos()).expect("a step fits in u64 nanoseconds");
        let jitter = Duration::from_nanos(
            self.jitter
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .gen_range(0..step),
        );
        let steps = u32::try_from(attempt.saturating_sub(1))
            .unwrap_or(u32::MAX)
            .min(MAX_STEPS);
        let pause = PAUSE_STEP * steps + jitter;
        assert!(
            pause < PAUSE_STEP * (MAX_STEPS + 1),
            "a pause stays under its bound"
        );
        pause
    }
}

/// The first moment the board cannot spell: 10000-01-01T00:00:00Z.
const PAST_9999_SECONDS: u64 = 253_402_300_800;

/// `time` to the second, as the board spells it.
///
/// # Errors
/// A time before 1970 or past 9999, which the board cannot spell.
pub fn timestamp_at(time: SystemTime) -> Result<Timestamp, BoardClockError> {
    let since = time.duration_since(UNIX_EPOCH).map_err(|before| {
        BoardClockError::OutOfRange(format!(
            "{} s before 1970-01-01T00:00:00Z",
            before.duration().as_secs_f64()
        ))
    })?;
    if since.as_secs() >= PAST_9999_SECONDS {
        return Err(BoardClockError::OutOfRange(format!(
            "{} s after 1970-01-01T00:00:00Z",
            since.as_secs()
        )));
    }
    let text = humantime::format_rfc3339_seconds(time).to_string();
    let timestamp =
        Timestamp::parse(&text).map_err(|_| BoardClockError::OutOfRange(text.clone()))?;
    assert_eq!(
        timestamp.as_str(),
        text,
        "the board spells the time as read"
    );
    Ok(timestamp)
}

impl BoardClock for SystemBoardClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        timestamp_at(SystemTime::now())
    }

    fn reference(&self) -> Option<Timestamp> {
        None
    }

    /// Blocks the calling thread for [`Self::pause_for`].
    fn pause(&self, attempt: usize) {
        std::thread::sleep(self.pause_for(attempt));
    }
}

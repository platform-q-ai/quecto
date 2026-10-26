//! A client's watch of a board server's feed (epic #2705 B10): each event
//! handed on once, the stream opened again from the last id whenever it
//! ends or is cut, an epoch already handed on not handed on again. A busy
//! server is tried again with a doubling backoff, within a time and a
//! count; a refusal or a malformed stream ends the watch at once; opens
//! that fail, or end with no event, end it after a few in a row. The
//! watch also ends, quietly, once its output has no reader left.
use std::sync::Arc;
use std::time::Duration;

use crate::application::project_board::ports::feed::{
    BoardStreams, FeedEvent, OpenRefused, StreamRead, WatchFlow, WatchPauses,
};

/// A watch's times and counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchTiming {
    /// The pause before a stream is opened again, until a stream's
    /// `retry:` says otherwise.
    pub pause: Duration,
    /// The shortest and longest pause between tries of a busy server.
    pub min_backoff: Duration,
    pub max_backoff: Duration,
    /// How long, and how many times at most, a busy server is tried again.
    pub busy_for: Duration,
    pub busy_tries: u32,
    /// Opens in a row that may fail, or end with no event.
    pub attempts: u32,
}

impl WatchTiming {
    /// Busy past a connection's 60 s lifetime, by which every stream
    /// holding a place has given it back.
    pub const PRODUCTION: Self = Self {
        pause: Duration::from_secs(1),
        min_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_secs(8),
        busy_for: Duration::from_secs(75),
        busy_tries: 32,
        attempts: 5,
    };
}

/// The highest busy backoff ceiling a watch takes.
const MAX_BACKOFF_CEILING: Duration = Duration::from_secs(3_600);

pub struct WatchBoard {
    streams: Box<dyn BoardStreams>,
    pauses: Arc<dyn WatchPauses>,
    timing: WatchTiming,
}

/// A busy server's tries so far: how many, the pauses slept (counted, so
/// nothing reads a clock), and the next pause.
#[derive(Debug, Clone, Copy)]
struct Busy {
    tries: u32,
    waited: Duration,
    backoff: Duration,
}

impl WatchBoard {
    pub fn new(
        streams: Box<dyn BoardStreams>,
        pauses: Arc<dyn WatchPauses>,
        timing: WatchTiming,
    ) -> Self {
        // A watch's times are its composition's constants: one out of range is
        // a bug, caught as it is made.
        assert!(timing.pause > Duration::ZERO, "a reopen never spins");
        assert!(
            timing.min_backoff > Duration::ZERO
                && timing.min_backoff <= timing.max_backoff
                && timing.max_backoff <= MAX_BACKOFF_CEILING,
            "a busy backoff never spins and has a ceiling of at most an hour"
        );
        Self {
            streams,
            pauses,
            timing,
        }
    }

    /// Hands `each` every event after `after` (none: from now) until it
    /// says stop, or `output_open` says the output has no reader left
    /// (asked before each open).
    ///
    /// # Errors
    /// Why the watch could not go on: a refusal, a malformed stream, a
    /// server busy past its bound, or opens failing in a row.
    pub fn watch(
        &self,
        after: Option<u64>,
        output_open: &mut dyn FnMut() -> bool,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<(), String> {
        let timing = self.timing;
        let (mut cursor, mut epoch) = (after, None);
        let (mut failed, mut pause, mut busy) = (0, timing.pause, None::<Busy>);
        loop {
            if !output_open() {
                return Ok(());
            }
            let failure = match self.streams.open(cursor) {
                Err(OpenRefused::Busy(why)) => {
                    let floor = pause.clamp(timing.min_backoff, timing.max_backoff);
                    let tried = busy.get_or_insert(Busy {
                        tries: 0,
                        waited: Duration::ZERO,
                        backoff: floor,
                    });
                    let left = timing.busy_for.saturating_sub(tried.waited);
                    match (tried.tries < timing.busy_tries, left > Duration::ZERO) {
                        (true, true) => {}
                        (_, _) => {
                            let waited = tried.waited.as_secs_f64();
                            return Err(format!("{why} (still busy after {waited:.1} s)"));
                        }
                    }
                    // Never past the bound: the last pause is what is left.
                    let slept = tried.backoff.min(left);
                    self.pauses.pause(slept);
                    *tried = Busy {
                        tries: tried.tries + 1,
                        waited: tried.waited + slept,
                        backoff: tried
                            .backoff
                            .saturating_mul(2)
                            .clamp(timing.min_backoff, timing.max_backoff),
                    };
                    continue;
                }
                Err(OpenRefused::Refused(why)) => return Err(why),
                Err(OpenRefused::Unreachable(why)) => Some(why),
                Ok(mut stream) => {
                    let mut heard = false;
                    loop {
                        match stream.read() {
                            StreamRead::Event { id, event } => {
                                (cursor, heard) = (Some(id), true);
                                match event {
                                    FeedEvent::Epoch { epoch: told } if epoch == Some(told) => {
                                        continue;
                                    }
                                    FeedEvent::Epoch { epoch: told } => epoch = Some(told),
                                    FeedEvent::Revision(_) | FeedEvent::Reset { .. } => {}
                                }
                                if each(event) == WatchFlow::Stop {
                                    return Ok(());
                                }
                            }
                            StreamRead::Ended => break,
                            StreamRead::Malformed(why) => return Err(why),
                        }
                    }
                    pause = stream.retry().unwrap_or(pause);
                    match heard {
                        true => {
                            (failed, busy) = (0, None);
                            None
                        }
                        false => Some("the board server's stream ended with no event".to_string()),
                    }
                }
            };
            if let Some(why) = failure {
                failed += 1;
                if failed >= timing.attempts {
                    return Err(format!("{why} ({failed} times in a row)"));
                }
            }
            if !output_open() {
                return Ok(());
            }
            self.pauses.pause(pause);
        }
    }
}

#[cfg(test)]
#[path = "watch_board_tests.rs"]
mod tests;

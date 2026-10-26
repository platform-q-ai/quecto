//! A follower's stall clock (epic #2705 B11c): the time since it was made,
//! on the monotonic clock, so a wall-clock step never shortens a stall.
use std::time::{Duration, Instant};

use crate::application::project_board::ports::replication::ReplicaClock;

pub struct MonotonicReplicaClock(Instant);

impl Default for MonotonicReplicaClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl ReplicaClock for MonotonicReplicaClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

//! A follower's stall clock (epic #2705 B11c): boot time, monotonic and
//! counting time suspended, so neither a wall-clock step nor a suspend
//! shortens a stall.
use std::time::Duration;

use crate::application::project_board::ports::replication::ReplicaClock;

/// Linux's boot time; macOS's monotonic clock counts sleep already.
#[cfg(target_os = "linux")]
const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
#[cfg(target_os = "macos")]
const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;

pub struct MonotonicReplicaClock;

impl ReplicaClock for MonotonicReplicaClock {
    fn now(&self) -> Duration {
        let mut at = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: a valid out-pointer and a clock this platform has.
        let read = unsafe { libc::clock_gettime(CLOCK, &mut at) };
        assert_eq!(read, 0, "the boot-time clock is read");
        let seconds = u64::try_from(at.tv_sec).unwrap_or_default();
        Duration::new(seconds, u32::try_from(at.tv_nsec).unwrap_or_default())
    }
}

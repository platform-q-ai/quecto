//! Shared existing held-lock fixture policy, not a production synchronization API.
//! The real drivers sleep; deterministic policy tests advance a fake timer.
use std::time::Duration;

pub fn timer_release(after: Duration, wait: impl FnOnce(Duration), release: impl FnOnce()) {
    wait(after);
    release();
}

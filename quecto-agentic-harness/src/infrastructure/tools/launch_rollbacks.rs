//! Launch work that must finish after its caller is gone (#2173): a
//! container create whose spawn was dropped, and the rollback of a
//! prepared launch that was never registered. Each runs as a detached
//! task counted here, so a run that stops can wait for them before the
//! runtime (and with it every unfinished task) is dropped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// One counted piece of launch work; dropping it ends the count.
#[derive(Debug)]
pub(in crate::infrastructure::tools) struct InFlight(());

impl InFlight {
    pub(in crate::infrastructure::tools) fn enter() -> Self {
        IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let before = IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(before > 0, "launch work counted out more than in");
    }
}

/// How often [`settled`] looks again.
const POLL: Duration = Duration::from_millis(50);

/// Wait until no launch work is in flight, for at most `limit`. True when
/// everything finished.
pub async fn settled(limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        if IN_FLIGHT.load(Ordering::SeqCst) == 0 {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
#[path = "launch_rollbacks_tests.rs"]
mod tests;

//! Every exit waits for launch rollbacks (#2173): dropping the runtime
//! would end them where they stand, leaving a container or an admission
//! scope behind.
use std::time::Duration;

/// How long an exit waits: a stopped create removing what it made (its
/// 3 s grace), or a made container's cleanup. The TUI's exit budget and a
/// parent's wait for an acknowledged child both count it.
pub(crate) const LAUNCH_ROLLBACK_LIMIT: Duration = Duration::from_secs(5);

/// Wait (bounded) for cancelled launches to roll back; false, with a
/// warning, when some were still running at the limit.
pub(crate) async fn await_launch_rollbacks(limit: Duration) -> bool {
    let settled = crate::infrastructure::tools::launch_rollbacks::settled(limit).await;
    if !settled {
        tracing::warn!(
            "a cancelled sub-agent launch was still rolling back at exit: `quecto container ls --all` lists an environment it left and `quecto container kill <ref>` ends it; a container whose create had not finished is not recorded and may need removing with the container runtime"
        );
    }
    settled
}

/// Held by a one-shot run over its runtime: whichever way the run ends,
/// dropping it waits for rollbacks before the runtime goes (#2173).
pub(crate) struct WaitForLaunchRollbacks<'a>(pub(crate) &'a tokio::runtime::Runtime);

impl Drop for WaitForLaunchRollbacks<'_> {
    fn drop(&mut self) {
        self.0
            .block_on(await_launch_rollbacks(LAUNCH_ROLLBACK_LIMIT));
    }
}

#[cfg(test)]
#[path = "launch_rollback_wait_tests.rs"]
mod tests;

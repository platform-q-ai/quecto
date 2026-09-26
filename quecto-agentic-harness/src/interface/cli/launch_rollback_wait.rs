//! Every exit waits for launch rollbacks (#2173): dropping the runtime
//! would end them where they stand, leaving a container or an admission
//! scope behind.
use std::time::Duration;

/// How long an exit waits: a container create a cancellation interrupted
/// finishes first, and only then is its container removed.
pub(crate) const LAUNCH_ROLLBACK_LIMIT: Duration = Duration::from_secs(60);

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

#[cfg(test)]
#[path = "launch_rollback_wait_tests.rs"]
mod tests;

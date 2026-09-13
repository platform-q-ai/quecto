//! Startup-failure cleanup of a spawned agent before any TUI exists (#1956):
//! the same leader-only termination as ordinary exit, with the settling
//! notice printed to stderr once instead of rendered.

/// Startup-failure cleanup of the primary spawn, before any TUI exists: the
/// same leader-only termination, with one stderr line if it takes > 1 s.
pub(super) async fn terminate_spawned_agent(child: &mut tokio::process::Child) {
    let identity = crate::shell::process::LeaderIdentity::capture(child.id());
    with_startup_exit_notice(crate::shell::process::terminate_leader(
        child,
        crate::shell::process::LeaderBudget::WORST_CASE,
        identity,
    ))
    .await;
}

/// Startup-failure cleanup of a watched spawn (no TUI yet, so the settling
/// notice goes to stderr once, after `SETTLING_NOTICE_AFTER`).
pub(super) async fn terminate_watched_at_startup(watch: &crate::shell::child_watch::ChildWatch) {
    let _ = with_startup_exit_notice(watch.terminate()).await;
}

/// Startup-failure exits run before the TUI exists and can take the whole
/// leader budget: say so on stderr once rather than block silently.
async fn with_startup_exit_notice<T>(exit: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(exit);
    tokio::select! {
        outcome = &mut exit => return outcome,
        _ = tokio::time::sleep(crate::shell::process::SETTLING_NOTICE_AFTER) => {
            eprintln!("waiting for the agent to exit…");
        }
    }
    exit.await
}

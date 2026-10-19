//! The harness's Tokio runtime (#1718, #2072): one builder shared by the
//! CLI's commands and the adapters that run a call on their own runtime,
//! so every one of them keeps the thread-exhaustion protection below.

/// Pool-wide keep-alive, in effect the process lifetime: why, see
/// [`build_tokio_runtime`].
pub(crate) const BLOCKING_THREAD_KEEP_ALIVE: std::time::Duration =
    std::time::Duration::from_secs(60 * 60 * 24 * 365);

/// The harness runs a current-thread runtime whose blocking pool keeps a
/// resident thread on purpose: Tokio's `spawn_blocking` panics (and with
/// `panic = "abort"` kills the process) when it must create a thread and the
/// OS answers EAGAIN, but merely queues that task when at least one blocking
/// thread already exists. Under a full pid cgroup (the case that was killing
/// swarm containers) that is the difference between a stalled call and a
/// dead container. Other spawn failures (ENOMEM) still panic.
///
/// The keep-alive is pool-wide: every blocking thread this process ever
/// creates is retained (bounded by Tokio's default cap of 512), each holding
/// one pid for the process lifetime. That is the accepted price.
///
/// A thread is probed with `std` first so that a process starting inside an
/// already exhausted cgroup gets an ordinary error, not an abort.
pub(crate) fn build_tokio_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    std::thread::Builder::new()
        .name("quecto-thread-probe".into())
        .spawn(|| {})?
        .join()
        .map_err(|_| std::io::Error::other("thread probe panicked"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_keep_alive(BLOCKING_THREAD_KEEP_ALIVE)
        .build()?;
    runtime.block_on(async {
        let _ = tokio::task::spawn_blocking(|| {}).await;
    });
    Ok(runtime)
}

#[cfg(test)]
#[path = "tokio_runtime_tests.rs"]
mod tests;

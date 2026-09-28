//! A child's pipe pumps on the [`OwnedChildSupervisor`]'s own runtime
//! (#2286), beside its reap task and its stderr tail.
//!
//! A pump spawned on whichever runtime happened to be current at launch
//! dies with that runtime while the child lives on: its reader would see
//! the end of a stream that has not ended, its writer would stop taking
//! input. On the supervisor's runtime a pump lives as long as the pipe it
//! serves.

use std::future::Future;

use super::owned_child_supervisor::OwnedChildSupervisor;

impl OwnedChildSupervisor {
    /// Run `pump` — the reader or writer of one owned child's pipe — on the
    /// supervisor's own runtime. Safe from any thread, inside or outside a
    /// runtime; the returned handle may be awaited or aborted from any.
    pub fn spawn_pump<F>(&self, pump: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(pump)
    }
}

#[cfg(test)]
#[path = "owned_child_supervisor_tasks_tests.rs"]
mod tests;

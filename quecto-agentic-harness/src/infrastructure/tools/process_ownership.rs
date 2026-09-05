//! A shared signaling lease for a locally owned child. Registry removal and
//! cloning do not transfer or extend the child's OS identity lifetime.

#[cfg(test)]
#[path = "process_ownership_tests.rs"]
mod tests;

use std::sync::{Arc, Mutex};
use std::task::Poll;

#[derive(Debug, Clone)]
pub(crate) struct ProcessOwnership(Arc<Mutex<bool>>);

impl ProcessOwnership {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(true)))
    }

    pub(crate) fn signal(&self, pid: u32, owner: super::process_tree::ProcessOwner) {
        self.dispatch(|| super::process_tree::terminate_owned_process_tree(pid, owner));
    }

    fn dispatch(&self, signal: impl FnOnce()) {
        let owned = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *owned {
            // Reaping cannot run while dispatch is in progress, including both
            // TERM and KILL of a local group. No existence check proves identity.
            signal();
        }
    }

    pub(crate) async fn wait(
        &self,
        child: &mut tokio::process::Child,
    ) -> std::io::Result<std::process::ExitStatus> {
        use std::future::Future;
        let mut wait = Box::pin(child.wait());
        std::future::poll_fn(|cx| {
            let mut owned = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let result = wait.as_mut().poll(cx);
            if matches!(result, Poll::Ready(_)) {
                // Invalidate every retained clone in the same critical section
                // that reaps. Even wait errors retire numeric signal authority.
                *owned = false;
            }
            result
        })
        .await
    }
}

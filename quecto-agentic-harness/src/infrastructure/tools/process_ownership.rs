//! A shared signaling lease for a locally owned child. Registry removal and
//! cloning do not transfer or extend the child's OS identity lifetime.
//!
//! The lease starts *unowned*: only the local launch path that actually
//! spawned a child may claim it ([`ProcessOwnership::launched`]). Entries
//! built by `SubagentEntry::new`/`with_identity`, merged from a child snapshot
//! or restored from persisted session state therefore carry a pid the harness
//! never signals (#1925: a fixture pid of 2 SIGTERMed a swarm coordinator).

#[cfg(test)]
#[path = "process_ownership_tests.rs"]
mod tests;

use std::sync::{Arc, Mutex};
use std::task::Poll;

#[derive(Debug, Clone)]
pub(crate) struct ProcessOwnership(Arc<Mutex<bool>>);

impl ProcessOwnership {
    /// A lease with no signal authority: the default for every entry whose
    /// process this harness did not launch itself.
    pub(crate) fn unowned() -> Self {
        Self(Arc::new(Mutex::new(false)))
    }

    /// Claim signal authority over a child this process just spawned. The
    /// child handle is required (not merely a pid) so authority can only be
    /// asserted by the code that holds the launched process.
    pub(crate) fn launched(child: &tokio::process::Child) -> Self {
        debug_assert!(
            child.id().is_some(),
            "launched ownership requires a live child"
        );
        Self(Arc::new(Mutex::new(child.id().is_some())))
    }

    /// Test seam: claim authority over a process the test spawned by other
    /// means (for example `std::process::Command`).
    #[cfg(test)]
    pub(crate) fn launched_for_test() -> Self {
        Self(Arc::new(Mutex::new(true)))
    }

    /// True while this lease still authorises signalling the child.
    #[cfg(test)]
    pub(crate) fn is_owned(&self) -> bool {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Signal the owned process tree. Returns `true` only when a signal was
    /// actually dispatched; unowned or already-reaped leases are skipped.
    pub(crate) fn signal(&self, pid: u32, owner: super::process_tree::ProcessOwner) -> bool {
        self.dispatch(|| super::process_tree::terminate_owned_process_tree(pid, owner))
    }

    fn dispatch(&self, signal: impl FnOnce()) -> bool {
        let owned = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *owned {
            // Reaping cannot run while dispatch is in progress, including both
            // TERM and KILL of a local group. No existence check proves identity.
            signal();
        }
        *owned
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

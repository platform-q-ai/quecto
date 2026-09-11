//! A shared signaling lease for a subagent's OS process. Registry removal and
//! cloning do not transfer or extend the child's OS identity lifetime.
//!
//! The lease is an allowlist of provenance (#1925). It starts [`Lease::Unowned`]
//! and only two facts grant signal authority:
//!
//! - [`ProcessOwnership::launched`]: this process spawned the child and holds
//!   its handle. The reaper retires the lease when it reaps.
//! - [`ProcessOwnership::reported`] with `same_namespace: true`: the pid was self-reported
//!   over a verified socket by a harness that provably shares our pid
//!   namespace (a host-local descendant merged from a host-local forwarding
//!   child, or a restored session child whose socket round-trip confirmed the
//!   persisted pid).
//!
//! Everything else — fixtures built by `SubagentEntry::new`/`with_identity`,
//! descendants reported from inside a container (another pid namespace),
//! restored rows whose pid was not confirmed — stays unowned and is never
//! signalled: a fixture pid of 2 once SIGTERMed a swarm coordinator.

#[cfg(test)]
#[path = "process_ownership_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "process_ownership_tests.rs"]
mod tests;

use std::sync::{Arc, Mutex};
use std::task::Poll;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lease {
    /// No signal authority (default, fixtures, foreign namespaces, reaped).
    Unowned,
    /// This process spawned the child and holds its handle.
    Launched,
    /// The pid was self-reported over a verified socket by a harness in our
    /// pid namespace. `same_namespace: false` records a report from another
    /// namespace (a container) and grants nothing.
    Reported { same_namespace: bool },
}

impl Lease {
    fn may_signal(self) -> bool {
        matches!(
            self,
            Lease::Launched
                | Lease::Reported {
                    same_namespace: true
                }
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessOwnership(Arc<Mutex<Lease>>);

impl ProcessOwnership {
    /// A lease with no signal authority: the default for every entry whose
    /// process provenance is unknown to this harness.
    pub(crate) fn unowned() -> Self {
        Self(Arc::new(Mutex::new(Lease::Unowned)))
    }

    /// Claim signal authority over a child this process just spawned. The
    /// child handle is required (not merely a pid) so authority can only be
    /// asserted by the code that holds the launched process.
    pub(crate) fn launched(child: &tokio::process::Child) -> Self {
        debug_assert!(
            child.id().is_some(),
            "launched ownership requires a live child"
        );
        let lease = if child.id().is_some() {
            Lease::Launched
        } else {
            Lease::Unowned
        };
        Self(Arc::new(Mutex::new(lease)))
    }

    /// A pid self-reported over a verified socket by a harness that shares
    /// our pid namespace (see module docs for the two admitted sources).
    pub(crate) fn reported(same_namespace: bool) -> Self {
        Self(Arc::new(Mutex::new(Lease::Reported { same_namespace })))
    }

    /// Test seam: claim launched authority over a process the test spawned by
    /// other means (for example `std::process::Command`).
    #[cfg(test)]
    pub(crate) fn launched_for_test() -> Self {
        Self(Arc::new(Mutex::new(Lease::Launched)))
    }

    #[cfg(test)]
    pub(crate) fn lease(&self) -> Lease {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// True while this lease still authorises signalling the child.
    #[cfg(test)]
    pub(crate) fn is_owned(&self) -> bool {
        self.lease().may_signal()
    }

    /// True while this lease is the launched-child lease held by a reaper.
    pub(crate) fn is_launched(&self) -> bool {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) == Lease::Launched
    }

    /// True when a harness reached through this entry provably shares our pid
    /// namespace, so pids IT reports for its own local children do too.
    pub(crate) fn is_same_namespace(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .may_signal()
    }

    /// Retire the lease IN PLACE so every retained clone (a cascade-removed
    /// copy, a shutdown drain) loses authority together with the registry row.
    /// Used when the harness that owned the process reports it reaped, so the
    /// pid may already belong to someone else. A launched lease is left to its
    /// reaper, which retires it in the same critical section that reaps.
    pub(crate) fn retire_reported(&self) {
        let mut lease = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*lease, Lease::Reported { .. }) {
            *lease = Lease::Unowned;
        }
    }

    /// Signal the owned process tree. Returns `true` only when a signal was
    /// actually dispatched; unowned, already-reaped and self/parent-targeting
    /// leases are skipped.
    pub(crate) fn signal(&self, pid: u32, owner: super::process_tree::ProcessOwner) -> bool {
        self.dispatch(|| super::process_tree::terminate_owned_process_tree(pid, owner))
    }

    fn dispatch(&self, signal: impl FnOnce() -> bool) -> bool {
        let lease = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if lease.may_signal() {
            // Reaping cannot run while dispatch is in progress, including both
            // TERM and KILL of a local group. No existence check proves identity.
            signal()
        } else {
            false
        }
    }

    pub(crate) async fn wait(
        &self,
        child: &mut tokio::process::Child,
    ) -> std::io::Result<std::process::ExitStatus> {
        use std::future::Future;
        let mut wait = Box::pin(child.wait());
        std::future::poll_fn(|cx| {
            let mut lease = self.0.lock().unwrap_or_else(|e| e.into_inner());
            let result = wait.as_mut().poll(cx);
            if matches!(result, Poll::Ready(_)) {
                // Invalidate every retained clone in the same critical section
                // that reaps. Even wait errors retire numeric signal authority.
                *lease = Lease::Unowned;
            }
            result
        })
        .await
    }
}

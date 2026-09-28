//! A child's stdout and stdin pumps run on the [`OwnedChildSupervisor`]'s
//! own runtime (#2286), beside its reap task and its stderr tail.
//!
//! A pump spawned on whichever runtime happened to be current at launch
//! dies with that runtime while the child lives on: its reader would see
//! the end of a stream that has not ended, its writer would stop taking
//! input. On the supervisor's runtime a pump lives as long as the pipe it
//! serves.
//!
//! Only these pipe shapes run there — never a caller's own future: the
//! supervisor's runtime has one worker thread, which reaps every child and
//! runs every termination; a caller's work (decoding a line, say) stays on
//! the caller's side ([`super::child_line_pipes`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::{ChildStdin, ChildStdout};
use tracing::instrument::WithSubscriber;

use super::child_line_pipes::{LineLimits, StdinLines, StdoutLines};
use super::owned_child_supervisor::{
    ChildHandleId, OwnedChildSupervisor, ProtocolOutcome, TerminationBudget, TerminationOutcome,
};

/// What a requester is told when its termination finished: the outcome
/// and how long it took. A synchronous record (a log line), never more
/// work on the supervisor's runtime.
pub type TerminationObserver = Box<dyn FnOnce(&TerminationOutcome, Duration) + Send>;

impl OwnedChildSupervisor {
    /// Read an owned child's `stdout` line by line within `limits`, on the
    /// supervisor's runtime. Safe from any thread, inside or outside a
    /// runtime.
    pub fn pump_stdout_lines(&self, stdout: ChildStdout, limits: LineLimits) -> StdoutLines {
        StdoutLines::pump(&self.handle, stdout, limits)
    }

    /// Write whole lines to an owned child's `stdin`, `queue` lines ahead,
    /// on the supervisor's runtime. Safe from any thread, inside or
    /// outside a runtime.
    pub fn pump_stdin_lines(&self, stdin: ChildStdin, queue: usize) -> StdinLines {
        StdinLines::pump(&self.handle, stdin, queue)
    }

    /// [`Self::request_termination`], telling `observe` how it ended. The
    /// termination (and so `observe`) runs under the requester's `tracing`
    /// subscriber. A handle no longer retained (the child was already
    /// reaped) is observed at once, on the caller, as
    /// [`TerminationOutcome::NoRetainedHandle`].
    pub fn request_termination_observed(
        self: &Arc<Self>,
        id: ChildHandleId,
        protocol: Pin<Box<dyn Future<Output = ProtocolOutcome> + Send>>,
        budget: TerminationBudget,
        observe: TerminationObserver,
    ) {
        let started = Instant::now();
        if self.retains(id) {
            let supervisor = Arc::clone(self);
            let termination = async move {
                let outcome = supervisor.terminate(id, protocol, budget).await;
                tracing::info!(handle = ?id, ?outcome, "owned child termination finished");
                observe(&outcome, started.elapsed());
            };
            self.handle.spawn(termination.with_current_subscriber());
        } else {
            observe(&TerminationOutcome::NoRetainedHandle, started.elapsed());
        }
    }
}

#[cfg(test)]
#[path = "owned_child_supervisor_tasks_tests.rs"]
mod tests;

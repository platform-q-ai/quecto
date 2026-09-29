//! A child's stdout and stdin pumps run on the [`OwnedChildSupervisor`]'s
//! own runtime (#2286), beside its reap task and its stderr tail; and a
//! requester may ask to have its child's termination logged.
//!
//! A pump spawned on whichever runtime happened to be current at launch
//! dies with that runtime while the child lives on: its reader would see
//! the end of a stream that has not ended, its writer would stop taking
//! input. On the supervisor's runtime a pump lives as long as the pipe it
//! serves.
//!
//! No caller code runs on the supervisor's runtime except the termination
//! protocol (#1935): it has one worker thread, which reaps every child and
//! runs every termination. A pump is a
//! [`super::super::child_line_pipes::PipeTask`] (a pipe and its
//! bookkeeping), and what a line means is decoded on the caller's side.
//! An observed termination is logged by the supervisor itself, from the
//! data-only [`TerminationObservation`] its requester hands over. This is
//! a child module of the supervisor's: it reaches the runtime only through
//! that file's private spawn helpers.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::{ChildStdin, ChildStdout};

use super::super::child_line_pipes::{LineLimits, StdinLines, StdoutLines};
use super::{
    ChildHandleId, OwnedChildSupervisor, ProtocolOutcome, TerminationBudget, TerminationOutcome,
};

/// The `tracing` target of the supervisor's own telemetry.
pub(crate) const TELEMETRY_TARGET: &str = "quecto::owned_child";

/// What a requester wants the supervisor to log once its child's
/// termination has finished: plain data, never code. The supervisor logs
/// it under [`TELEMETRY_TARGET`] with the last signal the termination
/// needed, whether the child is still running, and how long it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminationObservation {
    /// The kind of child, e.g. `"claude member"`.
    pub owner: &'static str,
    /// The requester's name for this child, e.g. a member's name. Never a
    /// secret: it is logged as it is.
    pub label: String,
}

impl TerminationObservation {
    /// Log how the observed termination ended.
    pub(super) fn log(&self, outcome: &TerminationOutcome, elapsed: Duration) {
        tracing::info!(
            target: TELEMETRY_TARGET,
            owner = %self.owner,
            label = %self.label,
            signal = %last_signal(outcome),
            still_running = matches!(outcome, TerminationOutcome::StillRunning { .. }),
            elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            "owned child termination observed"
        );
    }
}

/// The last signal a termination needed.
fn last_signal(outcome: &TerminationOutcome) -> &'static str {
    match outcome {
        TerminationOutcome::ExitedAfterKill { .. } | TerminationOutcome::StillRunning { .. } => {
            "KILL"
        }
        TerminationOutcome::ExitedAfterTerm { .. } => "TERM",
        TerminationOutcome::NoRetainedHandle
        | TerminationOutcome::AlreadyExited(_)
        | TerminationOutcome::ExitedAfterProtocol(_) => "none",
    }
}

impl OwnedChildSupervisor {
    /// Read an owned child's `stdout` line by line within `limits`, on the
    /// supervisor's runtime. Safe from any thread, inside or outside a
    /// runtime.
    pub fn pump_stdout_lines(&self, stdout: ChildStdout, limits: LineLimits) -> StdoutLines {
        let (lines, task) = StdoutLines::new(stdout, limits);
        lines.running(self.spawn_pipe_task(task))
    }

    /// Write whole lines to an owned child's `stdin`, `queue` lines ahead,
    /// on the supervisor's runtime. Safe from any thread, inside or
    /// outside a runtime.
    pub fn pump_stdin_lines(&self, stdin: ChildStdin, queue: usize) -> StdinLines {
        let (lines, task) = StdinLines::new(stdin, queue);
        // The writer ends by itself once its queue closes and drains.
        drop(self.spawn_pipe_task(task));
        lines
    }

    /// [`Self::request_termination`], after which the supervisor logs
    /// `observation` with how the termination ended. The termination runs
    /// under the requester's `tracing` subscriber, so the log reaches it.
    /// A handle no longer retained (the child was already reaped) is
    /// logged at once, on the caller, as
    /// [`TerminationOutcome::NoRetainedHandle`].
    pub(crate) fn request_termination_observed(
        self: &Arc<Self>,
        id: ChildHandleId,
        protocol: std::pin::Pin<Box<dyn Future<Output = ProtocolOutcome> + Send>>,
        budget: TerminationBudget,
        observation: TerminationObservation,
    ) {
        if self.retains(id) {
            self.spawn_termination(id, protocol, budget, Some(observation));
            return;
        }
        observation.log(&TerminationOutcome::NoRetainedHandle, Duration::ZERO);
    }
}

#[cfg(test)]
#[path = "tasks_tests.rs"]
mod tests;

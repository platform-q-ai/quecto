//! Watches the TUI-owned agent child process and records its exit diagnosis
//! (#1047).
//!
//! When the agent child dies mid-session (e.g. a panic-abort near a full
//! context window), the UDS connection simply closes — without this watcher
//! the TUI could only say "Agent disconnected" with no way to diagnose WHY.
//! The watcher owns the `Child`, reaps it in the background, and publishes a
//! human-readable exit description on a watch channel that the disconnect
//! notification surfaces.
//!
//! The watcher reaps the child in the background (`Child::wait`) and records
//! its exit diagnosis. Termination (#1956) signals the harness **leader
//! only**: SIGTERM, a bounded wait derived from the harness's fleet-teardown
//! budgets, SIGKILL of that one pid past the budget, then a read-only canary
//! over `/proc` — see [`crate::shell::process`]. No process group and no
//! descendant is ever signalled; the harness owns its subagent tree.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

pub use crate::shell::process::{LeaderBudget, LeaderEnd, LeaderIdentity, LeaderTermination};

/// Maximum stderr lines retained in a [`StderrTail`] ring buffer.
pub const STDERR_TAIL_MAX_LINES: usize = 20;

/// Bounded ring buffer of the agent child's most recent stderr lines (#1047).
///
/// The TUI keeps draining the child's stderr AFTER startup into this buffer
/// (see `shell::cli::spawn_stderr_drain`), so when the agent dies
/// mid-session (e.g. a panic-abort near a full context window under the
/// workspace `panic = "abort"`) the panic message is still available for the
/// disconnect diagnostics instead of being lost with the process.
#[derive(Debug, Clone)]
pub struct StderrTail {
    lines: Arc<Mutex<VecDeque<String>>>,
    /// Flips to `true` when the drain task has consumed stderr to EOF, so the
    /// disconnect path can wait for the panic message to land in the ring
    /// buffer instead of racing the drain task (#1051 final review).
    drained: Arc<watch::Sender<bool>>,
}

impl Default for StderrTail {
    fn default() -> Self {
        Self {
            lines: Arc::default(),
            drained: Arc::new(watch::channel(false).0),
        }
    }
}

impl StderrTail {
    /// Append a (already redacted/truncated) line, evicting the oldest once
    /// the ring is full.
    pub fn push(&self, line: String) {
        let mut lines = self.lines.lock().expect("stderr tail lock");
        if lines.len() == STDERR_TAIL_MAX_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// Record that the drain task has consumed the child's stderr to EOF —
    /// every line the child ever wrote (incl. a panic message) is now in the
    /// ring buffer.
    pub fn mark_drained(&self) {
        let _ = self.drained.send_replace(true);
    }

    /// Wait up to `timeout` for the drain to reach EOF (event-driven; resolves
    /// immediately once [`Self::mark_drained`] has run). Returns whether the
    /// drain completed within the window.
    pub async fn wait_drained(&self, timeout: Duration) -> bool {
        let mut rx = self.drained.subscribe();
        tokio::time::timeout(timeout, rx.wait_for(|drained| *drained))
            .await
            .is_ok()
    }

    /// Snapshot of the retained lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .expect("stderr tail lock")
            .iter()
            .cloned()
            .collect()
    }
}

/// A termination request: the leader-exit budget and where to report.
type TerminationRequest = (LeaderBudget, oneshot::Sender<LeaderTermination>);

/// Cloneable handle to the background watcher owning a spawned agent child.
#[derive(Debug, Clone)]
pub struct ChildWatch {
    exit_rx: watch::Receiver<Option<String>>,
    term_tx: mpsc::Sender<TerminationRequest>,
    stderr_tail: StderrTail,
    /// OS pid of the watched child at spawn time (registry durability, #1465).
    pid: Option<u32>,
}

/// Take ownership of the spawned agent child, reap it in the background, and
/// return a handle for reading its exit diagnosis and requesting termination.
///
/// `stderr_tail` is the ring buffer the child's post-startup stderr is being
/// drained into (#1047); the disconnect path reads it back through
/// [`ChildWatch::stderr_tail_lines`] to diagnose a panic-abort.
pub fn watch_child(mut child: tokio::process::Child, stderr_tail: StderrTail) -> ChildWatch {
    let (exit_tx, exit_rx) = watch::channel(None);
    let (term_tx, mut term_rx) = mpsc::channel::<TerminationRequest>(1);
    let pid = child.id();
    let identity = LeaderIdentity::capture(pid);
    tokio::spawn(async move {
        let mut exited = false;
        loop {
            tokio::select! {
                // `Child::wait` is cancel-safe: a termination request arriving
                // mid-wait leaves the handle intact for `terminate_leader`.
                status = child.wait(), if !exited => {
                    exited = true;
                    if let Ok(status) = status {
                        let _ = exit_tx.send(Some(describe_exit(status)));
                    }
                }
                request = term_rx.recv() => {
                    let Some((budget, done)) = request else { break; };
                    let outcome = if exited {
                        crate::shell::process::already_exited(identity)
                    } else {
                        crate::shell::process::terminate_leader(&mut child, budget, identity).await
                    };
                    let _ = done.send(outcome);
                    break;
                }
            }
        }
        // Drop leaves a live child alone (detach); Tokio reaps exited children.
    });
    ChildWatch {
        exit_rx,
        term_tx,
        stderr_tail,
        pid,
    }
}

impl ChildWatch {
    /// Test-only handle that never owns a real process.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn for_tests(pid: Option<u32>) -> Self {
        let (watch, _term_rx) = Self::for_tests_with_termination_probe(pid);
        watch
    }

    /// Test-only handle plus termination-request receiver. Dropping or never
    /// acknowledging the received oneshot simulates a stuck child watcher.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn for_tests_with_termination_probe(
        pid: Option<u32>,
    ) -> (Self, mpsc::Receiver<TerminationRequest>) {
        let (_exit_tx, exit_rx) = watch::channel(None);
        let (term_tx, term_rx) = mpsc::channel::<TerminationRequest>(1);
        (
            ChildWatch {
                exit_rx,
                term_tx,
                stderr_tail: StderrTail::default(),
                pid,
            },
            term_rx,
        )
    }

    /// OS pid captured when the watcher was created (may be gone if reaped).
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The recorded exit detail, if the child has exited and been reaped.
    pub fn exit_detail(&self) -> Option<String> {
        self.exit_rx.borrow().clone()
    }

    /// The child's most recent stderr lines (oldest first), captured by the
    /// post-startup drain (#1047). Empty when nothing was written.
    pub fn stderr_tail_lines(&self) -> Vec<String> {
        self.stderr_tail.lines()
    }

    /// Wait up to `timeout` for the stderr drain to reach EOF, so a snapshot
    /// of [`Self::stderr_tail_lines`] taken afterwards is guaranteed to
    /// include everything the dead child wrote — the exit diagnosis can land
    /// a beat before the drain task consumes the buffered panic message
    /// (#1051 final review). Returns whether the drain completed in time.
    pub async fn wait_stderr_drained(&self, timeout: Duration) -> bool {
        self.stderr_tail.wait_drained(timeout).await
    }

    /// Await the child's exit diagnosis for up to `timeout` (event-driven via
    /// the watch channel — no polling). The UDS stream usually closes a beat
    /// before the watcher reaps the child, so the disconnect path gives the
    /// diagnosis a short window to land rather than racing it.
    pub async fn wait_exit_detail(&self, timeout: Duration) -> Option<String> {
        let mut rx = self.exit_rx.clone();
        match tokio::time::timeout(timeout, rx.wait_for(Option::is_some)).await {
            Ok(Ok(detail)) => detail.clone(),
            _ => None,
        }
    }

    /// Terminate the leader with [`LeaderBudget::WORST_CASE`] (the roster is
    /// unknown to this caller) and wait for the watcher's report.
    pub async fn terminate(&self) -> Option<LeaderTermination> {
        self.terminate_with_budget(LeaderBudget::WORST_CASE).await
    }

    /// SIGTERM the leader, wait `budget.settle`, SIGTERM again and wait
    /// `budget.force`, SIGKILL that one pid past both, run the canary and
    /// report. `None` when the watcher is gone (the request cannot be
    /// delivered) or it never answers inside `budget.total()` plus a second
    /// — the caller is never held longer than that.
    pub async fn terminate_with_budget(&self, budget: LeaderBudget) -> Option<LeaderTermination> {
        let outer = budget.total().saturating_add(Duration::from_secs(1));
        tokio::time::timeout(outer, async {
            let (done_tx, done_rx) = oneshot::channel();
            self.term_tx.send((budget, done_tx)).await.ok()?;
            done_rx.await.ok()
        })
        .await
        .ok()
        .flatten()
    }
}

/// Human-readable description of how the agent child exited.
pub fn describe_exit(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(sig) = status.signal() {
        match signal_name(sig) {
            Some(name) => format!("agent process aborted: signal {sig} ({name})"),
            None => format!("agent process aborted: signal {sig}"),
        }
    } else if let Some(code) = status.code() {
        format!("agent process exited with code {code}")
    } else {
        "agent process exited with unknown status".to_string()
    }
}

/// Names for the signals an agent child plausibly dies from.
fn signal_name(sig: i32) -> Option<&'static str> {
    Some(match sig {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGTERM => "SIGTERM",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "child_watch_tests.rs"]
mod tests;

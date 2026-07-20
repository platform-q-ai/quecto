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
//! Termination also goes THROUGH the watcher (never by raw stored PID): the
//! watcher holds the un-reaped `Child`, so as long as a terminate request can
//! reach it the PID/PGID is pinned (alive or zombie) and cannot have been
//! recycled by an unrelated process. Once the watcher has reaped the child,
//! terminate requests fall back to a liveness-probed group signal
//! ([`crate::infrastructure::process::terminate_group_if_alive`]): sub-agents
//! share the agent's process group and can outlive it, so they must still be
//! cleaned up on TUI exit — but only after `kill(-pgid, 0)` confirms the
//! group has members, which pins the PGID against reuse (#1051 review:
//! PID-reuse race; final review: sub-agent orphan leak).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

/// Maximum stderr lines retained in a [`StderrTail`] ring buffer.
pub const STDERR_TAIL_MAX_LINES: usize = 20;

/// Bounded ring buffer of the agent child's most recent stderr lines (#1047).
///
/// The TUI keeps draining the child's stderr AFTER startup into this buffer
/// (see `interface::cli::spawn_stderr_drain`), so when the agent dies
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

/// Cloneable handle to the background watcher owning a spawned agent child.
#[derive(Debug, Clone)]
pub struct ChildWatch {
    exit_rx: watch::Receiver<Option<String>>,
    term_tx: mpsc::Sender<oneshot::Sender<()>>,
    stderr_tail: StderrTail,
}

/// Take ownership of the spawned agent child, reap it in the background, and
/// return a handle for reading its exit diagnosis and requesting termination.
///
/// `stderr_tail` is the ring buffer the child's post-startup stderr is being
/// drained into (#1047); the disconnect path reads it back through
/// [`ChildWatch::stderr_tail_lines`] to diagnose a panic-abort.
pub fn watch_child(mut child: tokio::process::Child, stderr_tail: StderrTail) -> ChildWatch {
    let (exit_tx, exit_rx) = watch::channel(None);
    let (term_tx, mut term_rx) = mpsc::channel::<oneshot::Sender<()>>(1);
    let pgid = child
        .id()
        .and_then(|raw| crate::infrastructure::process::checked_pid(raw).ok());
    tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => {
                if let Ok(status) = status {
                    let _ = exit_tx.send(Some(describe_exit(status)));
                }
                // The leader is reaped, but sub-agents spawned into its
                // process group can outlive it (the #1047 mid-session abort
                // case). Keep serving terminate requests with a
                // liveness-probed group signal so TUI exit still cleans them
                // up instead of leaking paid, long-lived processes.
                while let Some(done) = term_rx.recv().await {
                    if let Some(pgid) = pgid {
                        crate::infrastructure::process::terminate_group_if_alive(
                            pgid,
                            crate::infrastructure::process::TERMINATE_GRACE_MS,
                        )
                        .await;
                    }
                    let _ = done.send(());
                }
            }
            Some(done) = term_rx.recv() => {
                // `Child::wait` is cancel-safe, so losing the race here leaves
                // the child un-reaped: its PID/PGID is still pinned (alive or
                // zombie) and safe to signal via `terminate_child`, which
                // reaps it afterwards.
                crate::infrastructure::process::terminate_child(
                    &mut child,
                    crate::infrastructure::process::TERMINATE_GRACE_MS,
                )
                .await;
                let _ = done.send(());
            }
        }
    });
    ChildWatch {
        exit_rx,
        term_tx,
        stderr_tail,
    }
}

impl ChildWatch {
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

    /// Terminate the child's process group (SIGTERM → grace → SIGKILL) and
    /// wait for it to be reaped. After the watcher has reaped the child this
    /// falls back to a liveness-probed group signal: surviving group members
    /// (sub-agents) are still cleaned up, while an empty — and therefore
    /// possibly recycled — PGID is never signalled (#1051 review: PID-reuse
    /// race; final review: sub-agent orphan leak).
    pub async fn terminate(&self) {
        let (done_tx, done_rx) = oneshot::channel();
        if self.term_tx.send(done_tx).await.is_ok() {
            let _ = done_rx.await;
        }
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

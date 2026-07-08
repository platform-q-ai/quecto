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
//! terminate requests become no-ops instead of signalling a possibly-reused
//! PID (#1051 review: PID-reuse race).

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
#[derive(Debug, Clone, Default)]
pub struct StderrTail {
    lines: Arc<Mutex<VecDeque<String>>>,
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
    tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => {
                if let Ok(status) = status {
                    let _ = exit_tx.send(Some(describe_exit(status)));
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
    /// wait for it to be reaped. A no-op when the watcher has already reaped
    /// the child — signalling the stored PID after reap could hit an
    /// unrelated recycled process group (#1051 review).
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
mod tests {
    use super::*;

    fn spawn(script: &str) -> tokio::process::Child {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            // Match production: the child is its own process-group leader so
            // group-targeted termination has a real group to signal.
            .process_group(0);
        cmd.spawn().expect("spawn test child")
    }

    #[tokio::test]
    async fn records_signal_exit_with_name() {
        let watch = watch_child(spawn("kill -ABRT $$"), StderrTail::default());
        let detail = watch.wait_exit_detail(Duration::from_secs(5)).await;
        assert_eq!(
            detail.as_deref(),
            Some("agent process aborted: signal 6 (SIGABRT)")
        );
        assert_eq!(watch.exit_detail(), detail);
    }

    #[tokio::test]
    async fn records_nonzero_exit_code() {
        let watch = watch_child(spawn("exit 3"), StderrTail::default());
        let detail = watch.wait_exit_detail(Duration::from_secs(5)).await;
        assert_eq!(detail.as_deref(), Some("agent process exited with code 3"));
    }

    /// Termination goes through the watcher while it still owns the un-reaped
    /// child: the long-running child's group is terminated promptly.
    #[tokio::test]
    async fn terminate_kills_a_running_child_group() {
        let watch = watch_child(spawn("sleep 30"), StderrTail::default());
        tokio::time::timeout(Duration::from_secs(5), watch.terminate())
            .await
            .expect("terminate must complete well within the grace window");
    }

    /// #1051 review (PID-reuse race): once the watcher has reaped the child,
    /// terminate is a no-op — it must return immediately without signalling
    /// the (possibly recycled) stored PID.
    #[tokio::test]
    async fn terminate_after_reap_is_a_noop() {
        let watch = watch_child(spawn("exit 0"), StderrTail::default());
        assert!(
            watch
                .wait_exit_detail(Duration::from_secs(5))
                .await
                .is_some(),
            "child must be reaped first"
        );
        tokio::time::timeout(Duration::from_secs(1), watch.terminate())
            .await
            .expect("terminate after reap must be an immediate no-op");
    }

    /// The wait is event-driven: an exit already recorded resolves without
    /// consuming the timeout; a child that never exits resolves `None` at it.
    #[tokio::test]
    async fn wait_times_out_to_none_while_child_lives() {
        let watch = watch_child(spawn("sleep 30"), StderrTail::default());
        let detail = watch.wait_exit_detail(Duration::from_millis(50)).await;
        assert_eq!(detail, None);
        watch.terminate().await;
    }

    /// #1047: the stderr ring buffer keeps only the newest lines — a chatty
    /// child cannot grow it without bound, and the panic message (written
    /// last) is always retained.
    #[test]
    fn stderr_tail_keeps_only_newest_lines() {
        let tail = StderrTail::default();
        for i in 0..(STDERR_TAIL_MAX_LINES + 5) {
            tail.push(format!("line {i}"));
        }
        let lines = tail.lines();
        assert_eq!(lines.len(), STDERR_TAIL_MAX_LINES);
        assert_eq!(lines.first().map(String::as_str), Some("line 5"));
        assert_eq!(
            lines.last().map(String::as_str),
            Some(&*format!("line {}", STDERR_TAIL_MAX_LINES + 4))
        );
    }

    /// #1047: the watch handle exposes the drained stderr tail so the
    /// disconnect path can include it in the diagnostics.
    #[tokio::test]
    async fn watch_exposes_stderr_tail_lines() {
        let tail = StderrTail::default();
        tail.push("thread 'main' panicked at src/lib.rs:1: boom".to_string());
        let watch = watch_child(spawn("exit 0"), tail);
        assert_eq!(
            watch.stderr_tail_lines(),
            vec!["thread 'main' panicked at src/lib.rs:1: boom".to_string()]
        );
        watch.wait_exit_detail(Duration::from_secs(5)).await;
    }

    #[test]
    fn unknown_signal_has_no_name_suffix() {
        use std::os::unix::process::ExitStatusExt;
        let status = std::process::ExitStatus::from_raw(34); // signal 34 (real-time)
        assert_eq!(describe_exit(status), "agent process aborted: signal 34");
    }
}

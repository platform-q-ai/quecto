//! Agent-disconnect handling and dropped-event surfacing (#1047).
//!
//! When the agent process dies mid-session (e.g. a panic-abort near a full
//! context window), the UDS event stream simply closes. These methods keep
//! the failure diagnosable: the disconnect notification carries the owned
//! child's exit diagnosis, and oversized event lines the client had to drop
//! are surfaced instead of leaving the session silently looking frozen.

use super::*;

/// How long the disconnect path waits for the child-exit diagnosis to land
/// after the stream closes, before falling back to a bare "Agent disconnected".
const CHILD_EXIT_DETAIL_WINDOW: Duration = Duration::from_millis(500);

impl App {
    /// Attach the exit-diagnosis watch for a TUI-owned agent child (#1047),
    /// so a later disconnect can report WHY the agent went away.
    pub fn set_child_exit_watch(&mut self, watch: crate::shell::child_watch::ChildWatch) {
        self.child_exit_watch = Some(watch);
    }

    /// Test fixture: model a TUI that never showed the panel. #1047 pins the
    /// panel once it has been seen connected, so both flags must be cleared.
    #[cfg(test)]
    pub(super) fn clear_panel_for_tests(&mut self) {
        self.agent_connected = false;
        self.agent_ever_connected = false;
    }

    /// Handle the agent event stream closing (`client.recv()` → `None`).
    ///
    /// `exit_detail` carries the spawned agent child's exit diagnosis when the
    /// TUI owns the child process (e.g. "signal 6 (SIGABRT)" after a
    /// panic-abort near a full context window, #1047), so the disconnect
    /// notification can say WHY instead of a bare "Agent disconnected".
    pub(super) fn handle_agent_disconnected(&mut self, exit_detail: Option<String>) {
        self.agent_connected = false;
        self.agent_state.reset();
        self.master_session.running = false;
        self.spinner = None;
        self.master_session.chat.finalize_assistant();
        let mut message = match exit_detail {
            Some(detail) => format!("Agent disconnected — {detail}"),
            None => "Agent disconnected".to_string(),
        };
        // Include the child's drained stderr tail (#1047): under the workspace
        // `panic = "abort"` the panic message lands on stderr right before the
        // process dies — without it every recurrence is undiagnosable. The
        // newest line (usually the panic message) goes into the one-line
        // notification; the full tail goes into the transcript.
        let stderr_tail = self
            .child_exit_watch
            .as_ref()
            .map(|w| w.stderr_tail_lines())
            .unwrap_or_default();
        if let Some(last) = stderr_tail.last() {
            message = format!("{message} — last stderr: {last}");
            self.master_session.chat.add_entry(ChatEntry::Status {
                text: format!(
                    "Agent disconnected — recent agent stderr ({} lines):",
                    stderr_tail.len()
                ),
            });
            for line in &stderr_tail {
                self.master_session.chat.add_entry(ChatEntry::Status {
                    text: format!("  {line}"),
                });
            }
        }
        self.notify(&message, NotifyLevel::Error);
    }

    /// The event stream closed: diagnose the owned child's exit (if any) and
    /// run the disconnect handling with that detail (#1047).
    pub(super) async fn handle_agent_stream_closed(&mut self) {
        let detail = self.wait_child_exit_detail().await;
        // The exit diagnosis lands the moment the child is reaped, which can
        // be BEFORE the independent stderr-drain task consumes the buffered
        // panic message — give the drain the same bounded window so the
        // stderr snapshot below is complete, not racy (#1051 final review).
        if let Some(watch) = &self.child_exit_watch {
            watch.wait_stderr_drained(CHILD_EXIT_DETAIL_WINDOW).await;
        }
        self.handle_agent_disconnected(detail);
    }

    /// Best-effort read of the owned agent child's exit diagnosis. The stream
    /// usually closes a beat before the watcher reaps the child, so give the
    /// diagnosis a short window to land. Event-driven via the watcher's watch
    /// channel (#1051 review — no 20 ms poll loop): the common case resolves
    /// the moment the reap is recorded; only a child that closed its socket
    /// but stays alive costs the full (bounded, one-time) window.
    async fn wait_child_exit_detail(&self) -> Option<String> {
        let watch = self.child_exit_watch.as_ref()?;
        watch.wait_exit_detail(CHILD_EXIT_DETAIL_WINDOW).await
    }

    /// Surface newly-recorded oversized-event drops as a warning notification
    /// so the loss is visible instead of the session silently appearing
    /// frozen (#1047). Returns whether a notification was raised.
    pub(super) fn surface_dropped_oversized_events(&mut self) -> bool {
        let dropped = self.client.dropped_oversized_events();
        if dropped <= self.surfaced_oversized_drops {
            return false;
        }
        let new = dropped - self.surfaced_oversized_drops;
        self.surfaced_oversized_drops = dropped;
        let message = if new == 1 {
            "Dropped an oversized agent event — some output may be missing".to_string()
        } else {
            format!("Dropped {new} oversized agent events — some output may be missing")
        };
        self.notify(&message, NotifyLevel::Warning);
        true
    }
}

impl App {
    /// Surface a failed command send, attributed to the connection it
    /// happened on (#1460) so that with N per-tab connections the
    /// rollback/notice cannot be misrouted cross-tab.
    pub(super) fn handle_command_send_failure(&mut self, failure: CommandSendFailure) {
        let CommandSendFailure {
            command,
            error,
            connection,
        } = failure;
        self.rollback_failed_history_command(&connection, &command);
        let msg = format!(
            "Failed to send {} command on connection {connection}: {error}",
            command.kind()
        );
        self.notify(&msg, NotifyLevel::Error);
    }
}

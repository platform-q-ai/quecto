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

    /// Handle the agent event stream closing (the connection feed task's
    /// `SourcedEvent::Closed` sentinel, #1462).
    ///
    /// `exit_detail` carries the spawned agent child's exit diagnosis when the
    /// TUI owns the child process (e.g. "signal 6 (SIGABRT)" after a
    /// panic-abort near a full context window, #1047), so the disconnect
    /// notification can say WHY instead of a bare "Agent disconnected".
    /// Harness/test driver: the full disconnect (state flip + notice) in one
    /// step. Production paths go through `begin_/finish_agent_stream_closed`.
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn handle_agent_disconnected(&mut self, exit_detail: Option<String>) {
        self.mark_agent_disconnected();
        self.emit_agent_disconnected_notice(exit_detail);
    }

    /// Immediately mark the session disconnected: connection flag, run
    /// state, spinner, and streaming tail. Runs SYNCHRONOUSLY when the
    /// stream closes — the UI must never show a live session (nor accept
    /// prompts as deliverable) while the disconnect diagnosis is still
    /// resolving off-loop (#1470 review).
    fn mark_agent_disconnected(&mut self) {
        self.agent_connected = false;
        self.agent_state.reset();
        self.master_session.running = false;
        self.spinner = None;
        self.master_session.chat.finalize_assistant();
    }

    /// Emit the disconnect notification (and stderr-tail transcript entries,
    /// #1047) once the exit diagnosis — possibly deferred — is known.
    fn emit_agent_disconnected_notice(&mut self, exit_detail: Option<String>) {
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

    /// The event stream closed (`SourcedEvent::Closed` sentinel, #1462): dispatch
    /// the child-exit diagnosis OFF the select loop. When the TUI owns the
    /// agent child, the bounded diagnosis waits (#1047) run on a spawned task
    /// so a dying child can never stall event processing; the task reports
    /// the detail on the disconnect-diagnosis channel and the event loop
    /// completes the disconnect via [`Self::finish_agent_stream_closed`].
    ///
    /// The session state flip and dropped-event surfacing happen HERE,
    /// synchronously — only the diagnosis *text* (and its notification) is
    /// deferred, so the UI can never show a live session, and the oversized
    /// -drop warning can never be lost to an exit race, while the bounded
    /// child-exit waits run (#1470 review).
    ///
    /// Deferral is signalled solely via `disconnect_diag_pending` — the
    /// event loop's diagnosis arm and the harness both key off it, so there
    /// is exactly ONE deferral signal (#1470 r3, no drift-prone bool
    /// return). A `Closed` sentinel arriving while a diagnosis is already
    /// pending is a no-op: state is already flipped and a second diag task
    /// would duplicate the notification.
    pub(super) fn begin_agent_stream_closed(&mut self, tab: crate::shell::connection::TabId) {
        // Duplicate gate: the first sentinel flips `agent_connected`; any
        // later duplicate (ownerless attach connections included, where no
        // diagnosis latch is ever set) is a no-op — the exact once-per-
        // connection guarantee of the deleted gated select arm (#1470 r4).
        if !self.agent_connected {
            return;
        }
        self.disconnect_refusal_notified = false;
        self.surface_dropped_oversized_events();
        self.mark_agent_disconnected();
        let Some(watch) = self.child_exit_watch.clone() else {
            self.emit_agent_disconnected_notice(None);
            return;
        };
        self.disconnect_diag_pending = true;
        let tx = self.disconnect_diag_tx.clone();
        tokio::spawn(async move {
            // Best-effort read of the owned agent child's exit diagnosis. The
            // stream usually closes a beat before the watcher reaps the
            // child, so give the diagnosis a short window to land.
            // Event-driven via the watcher's watch channel (#1051 review —
            // no 20 ms poll loop): the common case resolves the moment the
            // reap is recorded; only a child that closed its socket but
            // stays alive costs the full (bounded, one-time) window.
            let detail = watch.wait_exit_detail(CHILD_EXIT_DETAIL_WINDOW).await;
            // The exit diagnosis lands the moment the child is reaped, which
            // can be BEFORE the independent stderr-drain task consumes the
            // buffered panic message — give the drain the same bounded
            // window so the stderr snapshot taken at completion is complete,
            // not racy (#1051 final review).
            watch.wait_stderr_drained(CHILD_EXIT_DETAIL_WINDOW).await;
            let _ = tx.send((tab, detail)).await;
        });
    }

    /// Complete the stream-closed disconnect: the session state was already
    /// flipped and drops surfaced synchronously in
    /// [`Self::begin_agent_stream_closed`]; this emits the notification with
    /// the (possibly deferred) exit diagnosis.
    /// Gated on the pending latch (#1470 r3): a session reset during the
    /// diagnosis window clears the latch, so the stale completion is
    /// dropped instead of dumping stderr into the fresh transcript.
    pub(super) fn finish_agent_stream_closed(
        &mut self,
        tab: crate::shell::connection::TabId,
        detail: Option<String>,
    ) {
        let _ = tab; // Keyed for phase 2 (#1463); N=1 uses one latch.
        if !self.disconnect_diag_pending {
            return;
        }
        self.disconnect_diag_pending = false;
        self.emit_agent_disconnected_notice(detail);
    }

    /// Surface newly-recorded oversized-event drops as a warning notification
    /// so the loss is visible instead of the session silently appearing
    /// frozen (#1047). Returns whether a notification was raised.
    pub(super) fn surface_dropped_oversized_events(&mut self) -> bool {
        let dropped = self.connection.dropped_oversized_events();
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
        self.rollback_failed_history_command(&connection, &command, false);
        let msg = format!(
            "Failed to send {} command on connection {connection}: {error}",
            command.kind()
        );
        // A burst (scroll issuing N stub recalls against a full writer)
        // would stack N identical toasts — show each distinct message once
        // while it is still visible (#1470 r5).
        if !self.notifications.messages().iter().any(|m| m == &msg) {
            self.notify(&msg, NotifyLevel::Error);
        }
    }
}

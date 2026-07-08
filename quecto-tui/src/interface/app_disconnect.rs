//! Agent-disconnect handling and dropped-event surfacing (#1047).
//!
//! When the agent process dies mid-session (e.g. a panic-abort near a full
//! context window), the UDS event stream simply closes. These methods keep
//! the failure diagnosable: the disconnect notification carries the owned
//! child's exit diagnosis, and oversized event lines the client had to drop
//! are surfaced instead of leaving the session silently looking frozen.

use super::*;

impl App {
    /// Attach the exit-diagnosis slot for a TUI-owned agent child (#1047), so
    /// a later disconnect can report WHY the agent went away.
    pub fn set_child_exit_watch(
        &mut self,
        slot: crate::infrastructure::child_watch::ExitDetailSlot,
    ) {
        self.child_exit_watch = Some(slot);
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
        let message = match exit_detail {
            Some(detail) => format!("Agent disconnected — {detail}"),
            None => "Agent disconnected".to_string(),
        };
        self.notify(&message, NotifyLevel::Error);
    }

    /// The event stream closed: diagnose the owned child's exit (if any) and
    /// run the disconnect handling with that detail (#1047).
    pub(super) async fn handle_agent_stream_closed(&mut self) {
        let detail = self.wait_child_exit_detail().await;
        self.handle_agent_disconnected(detail);
    }

    /// Best-effort read of the owned agent child's exit diagnosis. The stream
    /// usually closes a beat before the watcher reaps the child, so poll the
    /// slot briefly rather than racing it with a single read.
    async fn wait_child_exit_detail(&self) -> Option<String> {
        let slot = self.child_exit_watch.as_ref()?;
        for _ in 0..25 {
            if let Some(detail) = crate::infrastructure::child_watch::read_exit_detail(slot) {
                return Some(detail);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        None
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
            "Dropped an oversized agent event (>1 MiB) — some output may be missing".to_string()
        } else {
            format!("Dropped {new} oversized agent events (>1 MiB) — some output may be missing")
        };
        self.notify(&message, NotifyLevel::Warning);
        true
    }
}

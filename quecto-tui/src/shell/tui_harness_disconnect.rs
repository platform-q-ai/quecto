//! Harness drivers for the agent-disconnect diagnostics path (#1047): panel
//! pinning, the stream-closed disconnect handling, and the real child-exit
//! watcher integration. Used by the `tui_agent_disconnect_diagnostics.feature`
//! BDD steps.

use super::TuiHarness;

impl TuiHarness {
    /// Start a new master conversation through the production reset path.
    pub fn reset_master_session(&mut self) {
        self.app.reset_session("New session started");
    }

    /// Replace the master connection with a disconnected command channel.
    /// The replaced connection's feed task is aborted so the orphaned task
    /// cannot later inject a spurious `Closed` sentinel into the fan-in
    /// when the real socket drops (#1470 review).
    pub fn disconnect_master_commands(&mut self) {
        let old = std::mem::replace(
            &mut self.app.active_conn_mut().transport,
            crate::shell::connection::Connection::disconnected_for_tests(),
        );
        old.abort_feed();
    }

    /// Drain one command-send failure through the production handler.
    pub async fn handle_next_command_send_failure(&mut self) -> bool {
        let Ok(Some(failure)) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            self.app.command_send_failure_rx.recv(),
        )
        .await
        else {
            return false;
        };
        self.app.handle_command_send_failure(failure);
        true
    }

    /// Whether any notification is currently visible, rendered to text via the
    /// real notification stack so tests can assert the message content.
    pub fn notification_text(&mut self) -> String {
        use crate::components::component::Component;
        let w = self.width;
        self.app.notifications.render(w).join("\n")
    }

    /// Raw notification messages regardless of display expiry (#1067):
    /// content assertions must not race the 3s popup lifetime under
    /// concurrent-scenario scheduling delays.
    pub fn notification_messages(&self) -> Vec<String> {
        self.app.notifications.messages()
    }

    /// #1047: whether the persistent left (sub-agent) panel is shown.
    pub fn subagent_panel_visible(&self) -> bool {
        self.app.subagent_panel_visible()
    }

    /// #1047: whether the app still believes the agent is connected.
    pub fn agent_connected(&self) -> bool {
        self.app.active_conn().agent_connected
    }

    /// #1047: drive the production disconnect handling for an unexpectedly
    /// closed agent event stream (no owned-child diagnosis) and capture.
    pub fn agent_stream_closed(&mut self) -> &mut Self {
        self.app.handle_agent_disconnected(None);
        self.capture();
        self
    }

    /// #1047: attach a real child-exit watcher, then drive the production
    /// stream-closed path (which reads the exit diagnosis from it). Routes
    /// through the same `Closed`-sentinel completion protocol as every
    /// other sourced driver (#1470 review — no separate copy).
    pub async fn agent_stream_closed_with_child_watch(
        &mut self,
        watch: crate::shell::child_watch::ChildWatch,
    ) {
        self.deliver_closed_sentinel_with_child_watch(watch).await;
    }
}

//! Harness drivers for the agent-disconnect diagnostics path (#1047): panel
//! pinning, the stream-closed disconnect handling, and the real child-exit
//! watcher integration. Used by the `tui_agent_disconnect_diagnostics.feature`
//! BDD steps.

use super::TuiHarness;

impl TuiHarness {
    /// Whether any notification is currently visible, rendered to text via the
    /// real notification stack so tests can assert the message content.
    pub fn notification_text(&mut self) -> String {
        use crate::interface::component::Component;
        let w = self.width;
        self.app.notifications.render(w).join("\n")
    }

    /// #1047: whether the persistent left (sub-agent) panel is shown.
    pub fn subagent_panel_visible(&self) -> bool {
        self.app.subagent_panel_visible()
    }

    /// #1047: whether the app still believes the agent is connected.
    pub fn agent_connected(&self) -> bool {
        self.app.agent_connected
    }

    /// #1047: drive the production disconnect handling for an unexpectedly
    /// closed agent event stream (no owned-child diagnosis) and capture.
    pub fn agent_stream_closed(&mut self) -> &mut Self {
        self.app.handle_agent_disconnected(None);
        self.capture();
        self
    }

    /// #1047: attach a real child-exit watcher, then drive the production
    /// stream-closed path (which reads the exit diagnosis from it).
    pub async fn agent_stream_closed_with_child_watch(
        &mut self,
        watch: crate::infrastructure::child_watch::ChildWatch,
    ) {
        self.app.set_child_exit_watch(watch);
        self.app.handle_agent_stream_closed().await;
        self.capture();
    }
}

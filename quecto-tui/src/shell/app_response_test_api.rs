//! Test/harness API for pending-id arming and connection attachment.

use super::*;

#[cfg(any(test, feature = "test-harness"))]
impl App {
    /// Arm exact-pending attach correlation for a synthetic response delivery.
    pub fn test_arm_attach_backfill(&mut self, id: &str) {
        self.ac_mut().pending_attach_backfill_id = Some(id.to_string());
    }

    /// Arm exact-pending resume correlation for a synthetic response delivery.
    pub fn test_arm_resume_messages(&mut self, id: &str) {
        self.ac_mut().pending_resume_messages_id = Some(id.to_string());
    }

    /// Arm this client's own `resume_session` request id for a synthetic response.
    pub fn test_arm_resume_session(&mut self, id: &str) {
        self.ac_mut().pending_session_resume_id = Some(id.to_string());
    }

    /// The request id of the `/resume` answer still outstanding, if any.
    pub fn test_pending_session_resume(&self) -> Option<&str> {
        self.ac().pending_session_resume_id.as_deref()
    }

    /// Arm exact-pending rewind-refresh correlation for a synthetic response delivery.
    pub fn test_arm_rewind_refresh(&mut self, id: &str) {
        self.ac_mut().pending_rewind_refresh_id = Some(id.to_string());
    }

    /// Pending attach id (test inspection / capture after real mint).
    pub fn test_pending_attach_backfill_id(&self) -> Option<&str> {
        self.ac().pending_attach_backfill_id.as_deref()
    }

    /// Pending resume id (test inspection / capture after real mint).
    pub fn test_pending_resume_messages_id(&self) -> Option<&str> {
        self.ac().pending_resume_messages_id.as_deref()
    }

    /// Pending rewind-refresh id (test inspection / capture after real mint).
    pub fn test_pending_rewind_refresh_id(&self) -> Option<&str> {
        self.ac().pending_rewind_refresh_id.as_deref()
    }

    /// Swap the connection's transport (and owned child watch) for a connected one.
    #[cfg(test)]
    pub(crate) fn test_attach_connection(
        &mut self,
        transport: crate::shell::connection::Connection,
        child_watch: Option<crate::shell::child_watch::ChildWatch>,
    ) {
        let state = self.ac_mut();
        state.transport = transport;
        state.child_exit_watch = child_watch;
        state.agent_connected = true;
        state.agent_ever_connected = true;
    }
}

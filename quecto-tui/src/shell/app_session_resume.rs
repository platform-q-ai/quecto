//! `/resume` request and response handling: the request id owned by this tab,
//! the Coordinator clock reset on a session identity change, and the view
//! refresh every resume answer triggers (#1726). Child module of
//! `app_response`.
use super::super::*;

impl App {
    pub(crate) fn send_list_sessions(&mut self) {
        self.send_command(Command::ListSessions {
            id: Some(self.ac().namespaced_id("resume-list")),
        });
    }

    pub(crate) fn send_resume_session(&mut self, session: &str) {
        if session.trim().is_empty() {
            self.send_list_sessions();
            return;
        }
        let id = self
            .ac()
            .namespaced_id(&format!("resume-{}", super::super::app_events::uuid_like()));
        self.ac_mut().pending_session_resume_id = Some(id.clone());
        let sent = self.send_command(Command::ResumeSession {
            id: Some(id),
            session: session.trim().to_string(),
        });
        if !sent {
            self.ac_mut().pending_session_resume_id = None;
        }
    }

    /// Every `resume_session` answer refreshes the view (the agent's session
    /// changed for all its clients); only this tab's own answer settles the
    /// resume latches, so a foreign answer cannot cancel an in-flight resume.
    pub(crate) fn handle_resume_response(
        &mut self,
        id: Option<&str>,
        success: bool,
        data: Option<serde_json::Value>,
        error: Option<String>,
    ) {
        if self.is_owned_resume_response(id) {
            self.ac_mut().pending_session_resume_id = None;
            self.ac_mut().pending_session_resume = None;
        }
        if success {
            self.clear_message_recovery();
            self.handle_resume_success(data);
        } else {
            self.notify_response_error("Resume failed", error);
        }
    }

    fn is_owned_resume_response(&self, id: Option<&str>) -> bool {
        self.ac()
            .pending_session_resume_id
            .as_deref()
            .is_some_and(|pending| id == Some(pending))
    }

    fn handle_resume_success(&mut self, data: Option<serde_json::Value>) {
        let ack = data
            .as_ref()
            .map(crate::protocol::state_payloads::parse_resume_session);
        // A resume into a session other than the one this tab is showing
        // (including one it has not learned yet) is a session boundary for
        // the Coordinator clock. An answer without an identity is not.
        if let Some(key) = ack.as_ref().and_then(|ack| ack.session_key.as_deref()) {
            if self.ac().session_key.as_deref() != Some(key) {
                self.ac_mut()
                    .reset_coordinator_clock(tokio::time::Instant::now());
            }
            self.ac_mut().session_key = Some(key.to_owned());
        }
        let session = ack.map_or_else(|| "session".to_string(), |ack| ack.name);
        self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
        self.request_resumed_transcript();
        self.send_session_stats();
        // The agent resets session-scoped state (e.g. the effort override,
        // #1067) on resume_session; re-fetch so the footer tracks it.
        self.send_state_resync();
    }
}
